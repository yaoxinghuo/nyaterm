//! Bounded, memory-only diagnostics. Tickets discard events from replaced activations.
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 100;
const MAX_BYTES: usize = 64 * 1024;
const MAX_MESSAGE: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Idle,
    Starting,
    Running,
    Stopped,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub timestamp: u64,
    pub version: String,
    pub level: String,
    pub source: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub status: Status,
    pub version: String,
    pub last_error: Option<String>,
    pub logs: VecDeque<LogEntry>,
}
struct Record {
    generation: u64,
    snapshot: Snapshot,
}
#[derive(Default)]
struct State {
    next: u64,
    records: HashMap<String, Record>,
}
#[derive(Clone, Default)]
pub struct Diagnostics {
    state: Arc<Mutex<State>>,
}
#[derive(Clone)]
pub struct Ticket {
    id: String,
    generation: u64,
}

impl Diagnostics {
    pub fn begin(&self, id: &str, version: &str) -> Ticket {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.next += 1;
        let generation = state.next;
        let record = state.records.entry(id.into()).or_insert_with(|| Record {
            generation,
            snapshot: Snapshot {
                status: Status::Idle,
                version: version.into(),
                last_error: None,
                logs: VecDeque::new(),
            },
        });
        record.generation = generation;
        record.snapshot.status = Status::Starting;
        record.snapshot.version = version.into();
        record.snapshot.last_error = None;
        append(
            &mut record.snapshot,
            "info",
            "host",
            "Starting native backend",
        );
        Ticket {
            id: id.into(),
            generation,
        }
    }
    pub fn update(
        &self,
        ticket: &Ticket,
        status: Option<Status>,
        level: &str,
        source: &str,
        message: &str,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(record) = state
            .records
            .get_mut(&ticket.id)
            .filter(|r| r.generation == ticket.generation)
        {
            // Protocol failures remain errors when the process subsequently reports its exit.
            if let Some(status) = status
                && !(status == Status::Running && record.snapshot.status != Status::Starting)
            {
                record.snapshot.status = status;
            }
            append(&mut record.snapshot, level, source, message);
        }
    }
    pub fn ticket(&self, id: &str) -> Option<Ticket> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.records.get(id).map(|r| Ticket {
            id: id.into(),
            generation: r.generation,
        })
    }
    pub fn stop(&self, id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.next += 1;
        let generation = state.next;
        if let Some(record) = state.records.get_mut(id) {
            record.generation = generation;
            record.snapshot.status = Status::Stopped;
            append(
                &mut record.snapshot,
                "info",
                "host",
                "Native backend stopped",
            );
        }
    }
    pub fn snapshot(&self, id: &str, version: &str) -> Snapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .records
            .get(id)
            .map(|r| {
                let mut snapshot = r.snapshot.clone();
                if snapshot.version != version {
                    snapshot.version = version.into();
                    snapshot.status = Status::Stopped;
                    snapshot.last_error = None;
                }
                snapshot
            })
            .unwrap_or_else(|| Snapshot {
                status: Status::Idle,
                version: version.into(),
                last_error: None,
                logs: VecDeque::new(),
            })
    }
    pub fn clear(&self, id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(record) = state.records.get_mut(id) {
            record.snapshot.logs.clear();
            record.snapshot.last_error = None;
        }
    }
    pub fn remove(&self, id: &str) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .records
            .remove(id);
    }
}

fn append(snapshot: &mut Snapshot, level: &str, source: &str, message: &str) {
    // Scope tokens are UUIDs. Also mask common credential assignments, even in stderr.
    static UUID: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
            .unwrap()
    });
    static SECRET: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?i)(scopeToken|password|passwd|token|api[_-]?key|secret|authorization)[\s"']*[:=][\s"']*[^\s,;"'}]+"#).unwrap()
    });
    let message = SECRET.replace_all(message, "$1=[redacted]");
    let message = UUID.replace_all(&message, "[scope]");
    let mut message: String = message
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    if message.len() > MAX_MESSAGE {
        let mut end = MAX_MESSAGE - 3;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        message.push_str("...");
    }
    let level = if ["debug", "info", "warn", "error"].contains(&level) {
        level
    } else {
        "info"
    };
    if level == "error" {
        snapshot.last_error = Some(message.clone());
    }
    snapshot.logs.push_back(LogEntry {
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        version: snapshot.version.clone(),
        level: level.into(),
        source: source.chars().take(64).collect(),
        message,
    });
    while snapshot.logs.len() > MAX_ENTRIES
        || snapshot
            .logs
            .iter()
            .map(|entry| entry.message.len() + entry.source.len() + entry.version.len())
            .sum::<usize>()
            > MAX_BYTES
    {
        snapshot.logs.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_redacts_and_discards_old_activations() {
        let store = Diagnostics::default();
        let old = store.begin("example.test", "1.0.0");
        store.update(
            &old,
            Some(Status::Error),
            "error",
            "stderr",
            "password=hunter2 scopeToken=12345678-1234-1234-1234-123456789abc",
        );
        let snapshot = store.snapshot("example.test", "1.0.0");
        assert!(!snapshot.last_error.unwrap().contains("hunter2"));
        for _ in 0..110 {
            store.update(&old, None, "info", "sdk", &"测".repeat(2000));
        }
        let snapshot = store.snapshot("example.test", "1.0.0");
        assert!(snapshot.logs.len() <= 100);
        assert!(
            snapshot
                .logs
                .iter()
                .map(|e| e.message.len() + e.source.len() + e.version.len())
                .sum::<usize>()
                <= MAX_BYTES
        );
        store.stop("example.test");
        store.update(&old, Some(Status::Error), "error", "host", "late crash");
        assert_eq!(
            store.snapshot("example.test", "1.0.0").status,
            Status::Stopped
        );
        let new = store.begin("example.test", "2.0.0");
        store.update(&new, Some(Status::Running), "info", "host", "Started");
        store.update(&old, Some(Status::Error), "error", "host", "late crash");
        assert_eq!(
            store.snapshot("example.test", "2.0.0").status,
            Status::Running
        );
        store.clear("example.test");
        assert!(store.snapshot("example.test", "2.0.0").logs.is_empty());
        store.remove("example.test");
        store.update(&new, Some(Status::Error), "error", "host", "late crash");
        assert_eq!(store.snapshot("example.test", "2.0.0").status, Status::Idle);
    }
}
