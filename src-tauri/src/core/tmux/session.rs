//! tmux control-mode (`-CC`) session driver.
//!
//! When an SSH terminal channel switches to control mode, this loop owns the
//! channel and projects the tmux session onto the regular session model:
//! every tmux pane in the active window becomes a virtual session registered
//! in [`SessionManager`], so `write_to_session`, `resize_session`,
//! `close_session`, attach/ack and output flow control all work unchanged.
//!
//! - Pane output (`%output` / `%extended-output`) is octal-decoded and pushed
//!   to the pane's own `SessionOutputCoalescer` (i.e. `terminal-output-<id>`).
//! - Pane input is sent back as `send-keys -t %<id> -H <hex>`.
//! - Layout notifications rebuild the pane tree which is emitted to the
//!   frontend via the `tmux-session-state` event.
//! - `%exit` (after `detach` or a remote kill) hands the channel back to the
//!   regular SSH loop, which continues as a normal shell.

use super::control::{ControlMessage, ControlParser, LayoutCell, TmuxUiNode, parse_line};
use crate::core::terminal_session::TerminalOutputDecoder;
use crate::core::{
    DynamicTitleCapabilities, SessionCommand, SessionCommandReceiver, SessionHandle, SessionInfo,
    SessionManager, SessionOutputCoalescer, SessionType, now_session_started_at,
    session_command_channel,
};
use russh::{ChannelMsg, client};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::pin::Pin;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Sleep};

/// Event emitted (globally) whenever the tmux projection changes.
pub(crate) const TMUX_STATE_EVENT: &str = "tmux-session-state";

/// Input is batched over this interval so `send-keys` stays cheap even for
/// IME composition or paste bursts.
const INPUT_FLUSH_DELAY_MS: u64 = 4;
/// Max payload bytes per `send-keys` command line.
const SEND_KEYS_CHUNK_BYTES: usize = 1024;
/// Debounce for structural refreshes (`list-windows`/`list-panes`).
const STATE_REFRESH_DELAY_MS: u64 = 120;
/// Debounce for applying renderer resize requests towards tmux.
const RESIZE_APPLY_DELAY_MS: u64 = 40;
/// Buffer cap for output belonging to panes we have not mapped yet.
const PENDING_PANE_OUTPUT_MAX_BYTES: usize = 512 * 1024;
/// How long to wait for `%exit` after `detach-client` before giving up.
const DETACH_EXIT_TIMEOUT_MS: u64 = 3000;
/// Flow control: after a `%pause`, let the pane breathe, then continue it.
const PAUSE_CONTINUE_DELAY_MS: u64 = 250;

#[derive(Debug)]
pub(crate) enum ControlExit {
    /// tmux detached/exited: the channel is back at a normal shell; the
    /// caller should resume regular I/O and forward `leftover` bytes first.
    ReturnToShell { leftover: Vec<u8> },
    /// The channel or the whole session was closed.
    Closed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmuxWindowInfo {
    pub id: String,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmuxStatePayload {
    pub control_session_id: String,
    pub exited: bool,
    pub windows: Vec<TmuxWindowInfo>,
    pub active_window_id: Option<String>,
    pub active_pane_session_id: Option<String>,
    pub tree: Option<TmuxUiNode>,
}

/// One tmux pane projected as a virtual terminal session.
struct VirtualPane {
    /// Virtual session id registered in the [`SessionManager`].
    session_id: String,
    output: Arc<SessionOutputCoalescer>,
    decoder: TerminalOutputDecoder,
    /// Forwarder task draining this pane's command channel.
    forwarder: JoinHandle<()>,
    /// Output received before `capture-pane` seeding completed.
    pending_output: VecDeque<Vec<u8>>,
    seeded: bool,
}

/// Why a response block was queued (FIFO correlation with `%begin`/`%end`).
enum PendingKind {
    WindowsRefresh,
    PanesRefresh,
    CapturePane { pane: String },
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitMode {
    Detach,
    Close,
}

struct WindowState {
    name: String,
    layout: Option<LayoutCell>,
}

pub(crate) struct ControlSession<'a> {
    app: &'a AppHandle,
    control_session_id: &'a str,
    manager: &'a Arc<SessionManager>,
    encoding: String,
    connection_id: Option<String>,

    parser: ControlParser,
    /// Lines collected inside a `%begin`..`%end` block.
    block_lines: Option<Vec<String>>,
    /// Every command line written to the control channel produces exactly one
    /// `%begin`/`%end` (or `%error`) block, in send order. The FIFO therefore
    /// must be pushed for *every* line — internal queries push their kind so
    /// the response can be interpreted, everything else pushes `Ignore`.
    pending_responses: VecDeque<PendingKind>,

    windows: HashMap<String, WindowState>,
    panes: HashMap<String, VirtualPane>,
    pane_names: HashMap<String, String>,
    pane_windows: HashMap<String, String>,
    active_window: Option<String>,
    active_pane: Option<String>,

    pane_cmd_tx: mpsc::UnboundedSender<(String, SessionCommand)>,
    pane_cmd_rx: mpsc::UnboundedReceiver<(String, SessionCommand)>,

    /// Buffered keystrokes per pane, flushed on a short timer.
    input_buf: HashMap<String, Vec<u8>>,
    input_flush: Option<Pin<Box<Sleep>>>,
    /// Panes tmux paused via flow control; continued after a short delay.
    paused_panes: HashSet<String>,
    continue_deadline: Option<Pin<Box<Sleep>>>,

    refresh_deadline: Option<Pin<Box<Sleep>>>,
    resize_deadline: Option<Pin<Box<Sleep>>>,
    /// Desired (cols, rows) reported by the renderer per pane.
    desired_sizes: HashMap<String, (u32, u32)>,
    /// Client size currently applied to tmux (active window root size).
    client_size: Option<(u32, u32)>,
    /// Output received for panes we have not mapped yet (e.g. background
    /// windows), kept in case the pane becomes visible. Bounded.
    unknown_output: HashMap<String, VecDeque<Vec<u8>>>,

    exit_mode: Option<ExitMode>,
    exit_seen: bool,
    detach_deadline: Option<Pin<Box<Sleep>>>,
}

impl<'a> ControlSession<'a> {
    fn new(
        app: &'a AppHandle,
        control_session_id: &'a str,
        manager: &'a Arc<SessionManager>,
        encoding: &str,
        connection_id: Option<String>,
    ) -> Self {
        let (pane_cmd_tx, pane_cmd_rx) = mpsc::unbounded_channel();
        Self {
            app,
            control_session_id,
            manager,
            encoding: encoding.to_string(),
            connection_id,
            parser: ControlParser::new(),
            block_lines: None,
            pending_responses: VecDeque::new(),
            windows: HashMap::new(),
            panes: HashMap::new(),
            pane_names: HashMap::new(),
            pane_windows: HashMap::new(),
            active_window: None,
            active_pane: None,
            pane_cmd_tx,
            pane_cmd_rx,
            input_buf: HashMap::new(),
            input_flush: None,
            paused_panes: HashSet::new(),
            continue_deadline: None,
            refresh_deadline: None,
            resize_deadline: None,
            desired_sizes: HashMap::new(),
            client_size: None,
            unknown_output: HashMap::new(),
            exit_mode: None,
            exit_seen: false,
            detach_deadline: None,
        }
    }

    /// Write one command line and register its expected response block.
    async fn send_line(
        &mut self,
        channel: &mut russh::Channel<client::Msg>,
        kind: PendingKind,
        line: &str,
    ) -> bool {
        let mut data = line.as_bytes().to_vec();
        data.push(b'\n');
        if channel.data(data.as_slice()).await.is_ok() {
            self.pending_responses.push_back(kind);
            true
        } else {
            false
        }
    }

    /// Queue an internal query whose block output should be parsed.
    async fn send_query(
        &mut self,
        channel: &mut russh::Channel<client::Msg>,
        kind: PendingKind,
        line: &str,
    ) {
        self.send_line(channel, kind, line).await;
    }

    fn arm_refresh(&mut self) {
        if self.refresh_deadline.is_none() {
            self.refresh_deadline = Some(Box::pin(tokio::time::sleep(Duration::from_millis(
                STATE_REFRESH_DELAY_MS,
            ))));
        }
    }

    async fn refresh_lists(&mut self, channel: &mut russh::Channel<client::Msg>) {
        // `window_name` goes last: names may contain tabs/spaces.
        self.send_query(
            channel,
            PendingKind::WindowsRefresh,
            "list-windows -F '#{window_id}\t#{window_active}\t#{window_layout}\t#{window_name}'",
        )
        .await;
        self.send_query(
            channel,
            PendingKind::PanesRefresh,
            "list-panes -s -F '#{pane_id}\t#{window_id}\t#{pane_current_command}'",
        )
        .await;
    }

    async fn spawn_pane(&mut self, pane_id: &str) {
        if self.panes.contains_key(pane_id) {
            return;
        }
        let session_id = format!("{}::pane{}", self.control_session_id, pane_id);
        let (cmd_tx, mut cmd_rx) = session_command_channel(session_id.clone());
        let output = SessionOutputCoalescer::for_app(
            self.app.clone(),
            format!("terminal-output-{session_id}"),
            cmd_tx.clone(),
        );

        let parent = self
            .manager
            .session_info(self.control_session_id)
            .await
            .ok();
        let name = self
            .pane_names
            .get(pane_id)
            .cloned()
            .unwrap_or_else(|| format!("tmux {pane_id}"));
        let info = SessionInfo {
            id: session_id.clone(),
            name,
            session_type: SessionType::SSH,
            started_at: now_session_started_at(),
            connection_id: self.connection_id.clone(),
            connected: true,
            owner_window_label: parent
                .as_ref()
                .and_then(|info| info.owner_window_label.clone()),
            ai_execution_profile: parent
                .as_ref()
                .map(|info| info.ai_execution_profile)
                .unwrap_or_default(),
            injection_active: false,
            dynamic_title_capabilities: DynamicTitleCapabilities::default(),
            remote_file_browser_enabled: false,
            remote_stats_enabled: false,
            ssh_profile: parent.as_ref().and_then(|info| info.ssh_profile.clone()),
            ssh_runtime_mode: None,
        };
        self.manager
            .add_session(SessionHandle {
                info,
                cmd_tx,
                startup_input_barrier: None,
                ssh_config: None,
                ssh_handle: None,
                cwd: Arc::default(),
                remote_fs: None,
            })
            .await;

        let pane_key = pane_id.to_string();
        let forward_tx = self.pane_cmd_tx.clone();
        let forward_pane = pane_key.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if forward_tx.send((forward_pane.clone(), cmd)).is_err() {
                    break;
                }
            }
        });

        // Output that arrived before the pane was mapped is replayed after
        // the capture-pane seed, keeping chronology.
        let mut pending_output = VecDeque::new();
        if let Some(mut buffered) = self.unknown_output.remove(&pane_key) {
            pending_output.append(&mut buffered);
        }

        self.panes.insert(
            pane_key.clone(),
            VirtualPane {
                session_id: session_id.clone(),
                output,
                decoder: TerminalOutputDecoder::new(&self.encoding),
                forwarder,
                pending_output,
                seeded: false,
            },
        );

        tracing::info!(
            session_id = %self.control_session_id,
            pane = %pane_id,
            virtual_session = %session_id,
            "tmux pane mapped to virtual session"
        );
    }

    async fn seed_pane(&mut self, channel: &mut russh::Channel<client::Msg>, pane_id: &str) {
        // `-e` keeps escape sequences, `-p` prints to stdout, `-S -` starts at
        // the beginning of the pane's scrollback.
        let command = format!("capture-pane -ep -S - -t '{pane_id}'");
        self.send_query(
            channel,
            PendingKind::CapturePane {
                pane: pane_id.to_string(),
            },
            &command,
        )
        .await;
    }

    async fn drop_pane(&mut self, pane_id: &str) {
        let Some(pane) = self.panes.remove(pane_id) else {
            return;
        };
        pane.output.close();
        pane.forwarder.abort();
        self.manager.remove_session(&pane.session_id).await;
        let _ = self
            .app
            .emit(&format!("session-closed-{}", pane.session_id), ());
        self.desired_sizes.remove(pane_id);
        self.input_buf.remove(pane_id);
        self.paused_panes.remove(pane_id);
        self.unknown_output.remove(pane_id);
        self.pane_names.remove(pane_id);
        self.pane_windows.remove(pane_id);
        if self.active_pane.as_deref() == Some(pane_id) {
            self.active_pane = None;
        }
        tracing::info!(
            session_id = %self.control_session_id,
            pane = %pane_id,
            "tmux pane virtual session removed"
        );
    }

    async fn teardown_panes(&mut self) {
        let ids: Vec<String> = self.panes.keys().cloned().collect();
        for pane_id in ids {
            self.drop_pane(&pane_id).await;
        }
    }

    /// Fold a layout cell into the UI tree, mapping pane ids to virtual
    /// session ids. Panes that are not (yet) mapped render as placeholders.
    fn build_ui_tree(&self, cell: &LayoutCell) -> Option<TmuxUiNode> {
        if cell.is_leaf() {
            let pane_num = cell.pane?;
            let pane_id = format!("%{pane_num}");
            // Skip panes without a virtual session (e.g. an exited pane still
            // present in a stale layout) so splits collapse cleanly.
            let pane = self.panes.get(&pane_id)?;
            let name = self.pane_name(&pane_id);
            return Some(TmuxUiNode::Pane {
                pane_id,
                session_id: pane.session_id.clone(),
                name,
            });
        }

        let mut node: Option<TmuxUiNode> = None;
        for child in &cell.children {
            let Some(second) = self.build_ui_tree(child) else {
                continue;
            };
            node = Some(match node {
                None => second,
                Some(first) => {
                    // `first` occupies [child0.offset, child.offset); the
                    // pixel ratio mirrors where this child's offset lands
                    // inside the parent cell.
                    let span = if cell.columns {
                        cell.width
                    } else {
                        cell.height
                    };
                    let offset = if cell.columns { child.x } else { child.y };
                    let ratio = if span > 0 {
                        (f64::from(offset) / f64::from(span)).clamp(0.05, 0.95)
                    } else {
                        0.5
                    };
                    TmuxUiNode::Split {
                        direction: if cell.columns {
                            "vertical"
                        } else {
                            "horizontal"
                        },
                        ratio,
                        first: Box::new(first),
                        second: Box::new(second),
                    }
                }
            });
        }
        node
    }

    fn pane_name(&self, pane_id: &str) -> String {
        self.pane_names
            .get(pane_id)
            .filter(|name| !name.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("tmux {pane_id}"))
    }

    fn emit_state(&self) {
        let tree = self
            .active_window
            .as_ref()
            .and_then(|id| self.windows.get(id))
            .and_then(|window| window.layout.as_ref())
            .and_then(|layout| self.build_ui_tree(layout));

        let windows = {
            let mut list: Vec<TmuxWindowInfo> = self
                .windows
                .iter()
                .map(|(id, window)| TmuxWindowInfo {
                    id: id.clone(),
                    name: window.name.clone(),
                    active: Some(id) == self.active_window.as_ref(),
                })
                .collect();
            list.sort_by(|a, b| a.id.cmp(&b.id));
            list
        };

        let payload = TmuxStatePayload {
            control_session_id: self.control_session_id.to_string(),
            exited: false,
            windows,
            active_window_id: self.active_window.clone(),
            active_pane_session_id: self
                .active_pane
                .as_ref()
                .and_then(|pane| self.panes.get(pane))
                .map(|pane| pane.session_id.clone()),
            tree,
        };
        let _ = self.app.emit(TMUX_STATE_EVENT, &payload);
    }

    fn emit_exited(&self) {
        let payload = TmuxStatePayload {
            control_session_id: self.control_session_id.to_string(),
            exited: true,
            windows: Vec::new(),
            active_window_id: None,
            active_pane_session_id: None,
            tree: None,
        };
        let _ = self.app.emit(TMUX_STATE_EVENT, &payload);
    }

    /// Reconcile virtual panes with the active window's layout.
    async fn reconcile_panes(&mut self, channel: &mut russh::Channel<client::Msg>) {
        let layout = self
            .active_window
            .as_ref()
            .and_then(|id| self.windows.get(id))
            .and_then(|window| window.layout.clone());
        let wanted: HashSet<String> = layout
            .as_ref()
            .map(|layout| {
                layout
                    .pane_ids()
                    .iter()
                    .map(|id| format!("%{id}"))
                    .collect()
            })
            .unwrap_or_default();

        // Drop panes that vanished or belong to a different window.
        let existing: Vec<String> = self.panes.keys().cloned().collect();
        for pane_id in existing {
            if !wanted.contains(&pane_id) {
                self.drop_pane(&pane_id).await;
            }
        }
        // Spawn panes we don't know yet, then seed their scrollback.
        let mut spawned = Vec::new();
        for pane_id in &wanted {
            if !self.panes.contains_key(pane_id) {
                self.spawn_pane(pane_id).await;
                spawned.push(pane_id.clone());
            }
        }
        for pane_id in spawned {
            self.seed_pane(channel, &pane_id).await;
        }
        if let Some(layout) = layout {
            self.client_size = Some((layout.width, layout.height));
        }
        self.emit_state();
    }

    async fn handle_message(&mut self, channel: &mut russh::Channel<client::Msg>, line: String) {
        // Command-response blocks capture all lines between begin/end.
        if let Some(lines) = self.block_lines.as_mut() {
            let message = parse_line(&line);
            match message {
                Some(ControlMessage::End | ControlMessage::Error) => {
                    let collected = std::mem::take(lines);
                    let failed = matches!(message, Some(ControlMessage::Error));
                    self.block_lines = None;
                    let kind = self
                        .pending_responses
                        .pop_front()
                        .unwrap_or(PendingKind::Ignore);
                    self.handle_block(channel, kind, collected, failed).await;
                }
                Some(ControlMessage::Begin) => {
                    tracing::warn!("tmux control: nested %begin; discarding outer block");
                    self.block_lines = None;
                    self.pending_responses.pop_front();
                }
                Some(ControlMessage::Exit { .. }) => {
                    self.mark_exit();
                }
                _ => {
                    lines.push(line);
                }
            }
            return;
        }

        let Some(message) = parse_line(&line) else {
            return;
        };
        match message {
            ControlMessage::Begin => {
                self.block_lines = Some(Vec::new());
            }
            ControlMessage::End | ControlMessage::Error => {
                // Unsolicited end without a begin: ignore.
            }
            ControlMessage::Output { pane, data }
            | ControlMessage::ExtendedOutput { pane, data } => {
                self.route_output(&pane, data);
            }
            ControlMessage::LayoutChange { window, layout } => {
                let entry = self.windows.entry(window.clone()).or_insert(WindowState {
                    name: String::new(),
                    layout: None,
                });
                entry.layout = Some(layout);
                // If we have no active window yet (attach raced the
                // list-windows answer), assume this one is current.
                if self.active_window.is_none() {
                    self.active_window = Some(window.clone());
                }
                if self.active_window.as_deref() == Some(window.as_str()) {
                    self.reconcile_panes(channel).await;
                }
            }
            ControlMessage::WindowAdd { window } => {
                self.windows.entry(window).or_insert(WindowState {
                    name: String::new(),
                    layout: None,
                });
                self.arm_refresh();
            }
            ControlMessage::WindowClose { window } => {
                self.windows.remove(&window);
                if self.active_window.as_ref() == Some(&window) {
                    // tmux selects another window automatically; the follow-up
                    // notifications drive the rebuild.
                    self.active_window = None;
                }
                self.arm_refresh();
            }
            ControlMessage::WindowRenamed { window, name } => {
                if let Some(entry) = self.windows.get_mut(&window) {
                    entry.name = name;
                    self.emit_state();
                }
            }
            ControlMessage::WindowPaneChanged { window, pane } => {
                if self.active_window.as_ref() == Some(&window) {
                    self.active_pane = Some(pane);
                    self.emit_state();
                }
            }
            ControlMessage::PaneExited { pane } => {
                self.drop_pane(&pane).await;
                self.emit_state();
            }
            ControlMessage::SessionChanged { .. } => {
                // Attached to a different session: rebuild from scratch.
                self.windows.clear();
                self.pane_windows.clear();
                self.pane_names.clear();
                self.active_window = None;
                self.active_pane = None;
                self.unknown_output.clear();
                self.arm_refresh();
            }
            ControlMessage::SessionRenamed { .. } | ControlMessage::SessionsChanged => {
                self.arm_refresh();
            }
            ControlMessage::SessionWindowChanged { window } => {
                self.active_window = Some(window.clone());
                self.active_pane = None;
                self.arm_refresh();
                // Only reconcile when we already have a layout; otherwise a
                // follow-up layout-change / list-windows will drive it and we
                // avoid tearing panes down for a window we cannot render yet.
                if self
                    .windows
                    .get(&window)
                    .and_then(|window| window.layout.as_ref())
                    .is_some()
                {
                    self.reconcile_panes(channel).await;
                }
            }
            ControlMessage::PaneModeChanged { .. }
            | ControlMessage::ClientSessionChanged
            | ControlMessage::Ignored => {}
            ControlMessage::ClientDetached { client } => {
                tracing::info!(
                    session_id = %self.control_session_id,
                    client = %client,
                    "tmux client detached notification"
                );
            }
            ControlMessage::Pause { pane } => {
                // Flow control kicks in only if we enabled `pause-after`;
                // give the renderer a moment then let tmux continue.
                self.paused_panes.insert(pane);
                if self.continue_deadline.is_none() {
                    self.continue_deadline = Some(Box::pin(tokio::time::sleep(
                        Duration::from_millis(PAUSE_CONTINUE_DELAY_MS),
                    )));
                }
            }
            ControlMessage::Continue { pane } => {
                self.paused_panes.remove(&pane);
            }
            ControlMessage::ConfigError { text } | ControlMessage::Message { text } => {
                tracing::info!(
                    session_id = %self.control_session_id,
                    message = %text,
                    "tmux control server message"
                );
            }
            ControlMessage::Exit { .. } => {
                self.mark_exit();
            }
        }
    }

    fn mark_exit(&mut self) {
        self.exit_seen = true;
    }

    fn route_output(&mut self, pane: &str, data: Vec<u8>) {
        if let Some(virtual_pane) = self.panes.get_mut(pane) {
            // Not-yet-seeded panes keep raw bytes: decoding is stateful and
            // must see each byte exactly once.
            if virtual_pane.seeded {
                let decoded = virtual_pane.decoder.decode(&data);
                virtual_pane.output.push_owned(decoded);
            } else {
                virtual_pane.pending_output.push_back(data);
            }
        } else {
            // Pane output can arrive before its layout (attach, or panes in
            // background windows). Buffer a bounded amount keyed by pane id.
            let pending = self.unknown_output.entry(pane.to_string()).or_default();
            let total: usize = pending.iter().map(Vec::len).sum();
            if total < PENDING_PANE_OUTPUT_MAX_BYTES {
                pending.push_back(data);
            }
        }
    }

    async fn handle_block(
        &mut self,
        channel: &mut russh::Channel<client::Msg>,
        kind: PendingKind,
        lines: Vec<String>,
        failed: bool,
    ) {
        match kind {
            PendingKind::WindowsRefresh => {
                self.apply_window_list(lines);
                self.reconcile_panes(channel).await;
            }
            PendingKind::PanesRefresh => {
                for line in lines {
                    let mut parts = line.split('\t');
                    let (Some(pane), Some(window), Some(name)) =
                        (parts.next(), parts.next(), parts.next())
                    else {
                        continue;
                    };
                    self.pane_windows
                        .insert(pane.to_string(), window.to_string());
                    self.pane_names.insert(pane.to_string(), name.to_string());
                }
                self.emit_state();
            }
            PendingKind::CapturePane { pane } => {
                if let Some(virtual_pane) = self.panes.get_mut(&pane) {
                    if !failed && !lines.is_empty() {
                        let mut decoder = TerminalOutputDecoder::new(&self.encoding);
                        let text = decoder.decode(lines.join("\r\n").as_bytes());
                        virtual_pane.output.push_owned(text);
                    }
                    virtual_pane.seeded = true;
                    while let Some(data) = virtual_pane.pending_output.pop_front() {
                        let decoded = virtual_pane.decoder.decode(&data);
                        virtual_pane.output.push_owned(decoded);
                    }
                }
            }
            PendingKind::Ignore => {}
        }
    }

    fn apply_window_list(&mut self, lines: Vec<String>) {
        for line in lines {
            let mut parts = line.splitn(4, '\t');
            let (Some(id), Some(active), Some(layout_str)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let name = parts.next().unwrap_or_default();
            let entry = self.windows.entry(id.to_string()).or_insert(WindowState {
                name: String::new(),
                layout: None,
            });
            entry.name = name.to_string();
            if let Some((layout, _)) = super::control::parse_layout(layout_str) {
                entry.layout = Some(layout);
            }
            if active == "1" {
                self.active_window = Some(id.to_string());
            }
        }
        self.emit_state();
    }

    async fn flush_input(&mut self, channel: &mut russh::Channel<client::Msg>) {
        let panes: Vec<String> = self.input_buf.keys().cloned().collect();
        for pane_id in panes {
            let data = self.input_buf.remove(&pane_id).unwrap_or_default();
            if data.is_empty() {
                continue;
            }
            for chunk in data.chunks(SEND_KEYS_CHUNK_BYTES) {
                let mut line = String::from("send-keys -H -t ");
                line.push_str(&pane_id);
                for byte in chunk {
                    let _ = write!(line, " {byte:02x}");
                }
                self.send_line(channel, PendingKind::Ignore, &line).await;
            }
        }
        self.input_flush = None;
    }

    async fn apply_resize(&mut self, channel: &mut russh::Channel<client::Msg>) {
        let Some(window) = self.active_window.as_ref() else {
            return;
        };
        let Some(layout) = self
            .windows
            .get(window)
            .and_then(|window| window.layout.clone())
        else {
            return;
        };

        // Client size = layout root with each leaf's desired size substituted.
        let (width, height) = required_size(&layout, &self.desired_sizes);
        let current = self.client_size.unwrap_or((layout.width, layout.height));
        if (width, height) != current {
            self.send_line(
                channel,
                PendingKind::Ignore,
                &format!("refresh-client -C {width}x{height}"),
            )
            .await;
            self.client_size = Some((width, height));
        }
        for (pane_id, (cols, rows)) in self.desired_sizes.clone() {
            let Some(pane_num) = pane_id.strip_prefix('%').and_then(|id| id.parse().ok()) else {
                continue;
            };
            let actual = layout
                .find_pane(pane_num)
                .map(|cell| (cell.width, cell.height));
            if actual != Some((cols, rows)) {
                self.send_line(
                    channel,
                    PendingKind::Ignore,
                    &format!("resize-pane -t '{pane_id}' -x {cols} -y {rows}"),
                )
                .await;
            }
        }
    }

    async fn continue_paused(&mut self, channel: &mut russh::Channel<client::Msg>) {
        let panes: Vec<String> = self.paused_panes.iter().cloned().collect();
        for pane in panes {
            self.send_line(
                channel,
                PendingKind::Ignore,
                &format!("refresh-client -A '{pane}:continue'"),
            )
            .await;
        }
        self.paused_panes.clear();
        self.continue_deadline = None;
    }

    async fn handle_pane_command(
        &mut self,
        channel: &mut russh::Channel<client::Msg>,
        pane_id: String,
        command: SessionCommand,
    ) {
        match command {
            SessionCommand::Write { data, .. } => {
                self.input_buf
                    .entry(pane_id)
                    .or_default()
                    .extend_from_slice(&data);
                if self.input_flush.is_none() {
                    self.input_flush = Some(Box::pin(tokio::time::sleep(Duration::from_millis(
                        INPUT_FLUSH_DELAY_MS,
                    ))));
                }
            }
            SessionCommand::Resize { cols, rows } => {
                self.desired_sizes.insert(pane_id, (cols, rows));
                if self.resize_deadline.is_none() {
                    self.resize_deadline = Some(Box::pin(tokio::time::sleep(
                        Duration::from_millis(RESIZE_APPLY_DELAY_MS),
                    )));
                }
            }
            SessionCommand::AttachConfirmed { ack } => {
                if let Some(pane) = self.panes.get(&pane_id) {
                    pane.output.attach_confirmed(ack);
                } else {
                    let _ = ack.send(());
                }
            }
            SessionCommand::DetachRenderer => {
                if let Some(pane) = self.panes.get(&pane_id) {
                    pane.output.detach();
                }
            }
            SessionCommand::AckOutput { bytes } => {
                if let Some(pane) = self.panes.get(&pane_id) {
                    pane.output.ack(bytes);
                }
            }
            SessionCommand::Close => {
                // Closing one pane leaf kills the tmux pane; the resulting
                // layout-change rebuilds the tree without it.
                self.send_line(
                    channel,
                    PendingKind::Ignore,
                    &format!("kill-pane -t '{pane_id}'"),
                )
                .await;
                self.drop_pane(&pane_id).await;
                self.emit_state();
            }
            SessionCommand::TmuxCommand { line } => {
                let clean = line.trim_matches(['\r', '\n']).to_string();
                if !clean.is_empty() {
                    self.send_line(channel, PendingKind::Ignore, &clean).await;
                }
            }
            SessionCommand::TmuxDetach => {
                self.request_detach(channel, ExitMode::Detach).await;
            }
            SessionCommand::PauseOutput
            | SessionCommand::ResumeOutput
            | SessionCommand::CancelCapture { .. }
            | SessionCommand::ZmodemAcceptDownload { .. }
            | SessionCommand::ZmodemAcceptUpload { .. }
            | SessionCommand::ZmodemCancel => {}
            SessionCommand::CaptureExec { result_tx, .. } => {
                // AI command capture is not supported inside tmux panes.
                drop(result_tx);
            }
        }
    }

    async fn request_detach(&mut self, channel: &mut russh::Channel<client::Msg>, mode: ExitMode) {
        if self.exit_mode.is_some() {
            return;
        }
        self.exit_mode = Some(mode);
        let _ = self
            .send_line(channel, PendingKind::Ignore, "detach-client")
            .await;
        self.detach_deadline = Some(Box::pin(tokio::time::sleep(Duration::from_millis(
            DETACH_EXIT_TIMEOUT_MS,
        ))));
    }

    /// Cleanup after `%exit` or channel termination: drop all virtual panes.
    async fn teardown(&mut self) {
        self.teardown_panes().await;
        self.emit_exited();
    }
}

/// tmux layout cell sizes are the client's truth. Compute the client size
/// needed so every pane can reach the size its renderer asked for.
fn required_size(cell: &LayoutCell, desired: &HashMap<String, (u32, u32)>) -> (u32, u32) {
    if let Some(pane) = cell.pane {
        if let Some(&(cols, rows)) = desired.get(&format!("%{pane}")) {
            return (cols, rows);
        }
        return (cell.width, cell.height);
    }
    let mut width = 0u32;
    let mut height = 0u32;
    let mut count = 0u32;
    for child in &cell.children {
        let (cw, ch) = required_size(child, desired);
        if cell.columns {
            width = width.saturating_add(cw);
            height = height.max(ch);
        } else {
            width = width.max(cw);
            height = height.saturating_add(ch);
        }
        count += 1;
    }
    // tmux draws a one-cell divider between siblings.
    let borders = count.saturating_sub(1);
    if cell.columns {
        (width.saturating_add(borders), height)
    } else {
        (width, height.saturating_add(borders))
    }
}

/// Drive the control session until `%exit` or channel termination.
pub(crate) async fn run_control_session(
    app: &AppHandle,
    session_id: &str,
    manager: &Arc<SessionManager>,
    channel: &mut russh::Channel<client::Msg>,
    cmd_rx: &mut SessionCommandReceiver,
    initial: &[u8],
    encoding: &str,
    connection_id: Option<String>,
) -> ControlExit {
    let mut session = ControlSession::new(app, session_id, manager, encoding, connection_id);

    tracing::info!(
        session_id = %session_id,
        "SSH channel switched to tmux control mode"
    );

    // Bootstrap: adopt the client's size management and get a deterministic
    // picture of windows/panes (attach notifications alone are not enough).
    session
        .send_line(
            channel,
            PendingKind::Ignore,
            "refresh-client -f pause-after=2",
        )
        .await;
    session.refresh_lists(channel).await;

    // Feed the bytes that arrived together with the first notification.
    for line in session.parser.feed(initial) {
        session.handle_message(channel, line).await;
        if session.exit_seen {
            break;
        }
    }

    let exit = loop {
        if session.exit_seen {
            break match session.exit_mode {
                Some(ExitMode::Close) => ControlExit::Closed,
                _ => ControlExit::ReturnToShell {
                    leftover: session.parser.take_pending(),
                },
            };
        }

        tokio::select! {
            biased;

            () = async {
                if let Some(deadline) = session.detach_deadline.as_mut() {
                    deadline.as_mut().await;
                }
            }, if session.detach_deadline.is_some() => {
                // detach-client never produced %exit: fall back to closing.
                break ControlExit::Closed;
            }

            () = async {
                if let Some(deadline) = session.input_flush.as_mut() {
                    deadline.as_mut().await;
                }
            }, if session.input_flush.is_some() => {
                session.flush_input(channel).await;
            }

            () = async {
                if let Some(deadline) = session.refresh_deadline.as_mut() {
                    deadline.as_mut().await;
                }
            }, if session.refresh_deadline.is_some() => {
                session.refresh_deadline = None;
                session.refresh_lists(channel).await;
            }

            () = async {
                if let Some(deadline) = session.resize_deadline.as_mut() {
                    deadline.as_mut().await;
                }
            }, if session.resize_deadline.is_some() => {
                session.resize_deadline = None;
                session.apply_resize(channel).await;
            }

            () = async {
                if let Some(deadline) = session.continue_deadline.as_mut() {
                    deadline.as_mut().await;
                }
            }, if session.continue_deadline.is_some() => {
                session.continue_deadline = None;
                session.continue_paused(channel).await;
            }

            command = cmd_rx.recv() => {
                match command {
                    Some(SessionCommand::TmuxCommand { line }) => {
                        let clean = line.trim_matches(['\r', '\n']);
                        if !clean.is_empty() {
                            session
                                .send_line(channel, PendingKind::Ignore, clean)
                                .await;
                        }
                    }
                    Some(SessionCommand::TmuxDetach) => {
                        session.request_detach(channel, ExitMode::Detach).await;
                    }
                    Some(SessionCommand::Close) => {
                        session.request_detach(channel, ExitMode::Close).await;
                    }
                    Some(SessionCommand::AttachConfirmed { ack }) => {
                        let _ = ack.send(());
                    }
                    Some(
                        SessionCommand::DetachRenderer
                        | SessionCommand::Write { .. }
                        | SessionCommand::Resize { .. }
                        | SessionCommand::PauseOutput
                        | SessionCommand::ResumeOutput
                        | SessionCommand::AckOutput { .. }
                        | SessionCommand::CancelCapture { .. }
                        | SessionCommand::ZmodemAcceptDownload { .. }
                        | SessionCommand::ZmodemAcceptUpload { .. }
                        | SessionCommand::ZmodemCancel,
                    ) => {}
                    Some(SessionCommand::CaptureExec { result_tx, .. }) => {
                        drop(result_tx);
                    }
                    None => break ControlExit::Closed,
                }
            }

            command = session.pane_cmd_rx.recv() => {
                if let Some((pane_id, cmd)) = command {
                    session.handle_pane_command(channel, pane_id, cmd).await;
                }
            }

            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        for line in session.parser.feed(&data) {
                            session.handle_message(channel, line).await;
                        }
                    }
                    None | Some(ChannelMsg::Eof | ChannelMsg::Close) => {
                        break ControlExit::Closed;
                    }
                    _ => {}
                }
            }
        }
    };

    session.teardown().await;
    exit
}
