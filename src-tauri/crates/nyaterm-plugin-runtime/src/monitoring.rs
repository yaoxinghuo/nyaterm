//! Demand-driven, shared monitor actors. No Tauri or SSH dependency.
use crate::{Result, invalid};
use async_trait::async_trait;
use nyaterm_gpu::RemoteGpuOverview;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MonitorKey {
    pub plugin_id: String,
    pub version: String,
    pub window: String,
    pub session: String,
    pub monitor_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSnapshot {
    pub revision: u64,
    pub session_id: String,
    pub overview: Option<RemoteGpuOverview>,
    pub error: bool,
    pub refreshing: bool,
    pub paused: bool,
}

#[async_trait]
pub trait Collector: Send + Sync {
    async fn collect(&self, scope: &str) -> Result<Value>;
    fn status(&self, _message: &str) {}
}

struct Demand {
    token: String,
    cancellation: CancellationToken,
    interval: Duration,
}

pub struct MonitorTask {
    demands: Mutex<HashMap<String, Demand>>,
    paused: Mutex<bool>,
    manual: Mutex<bool>,
    in_flight: Mutex<Option<CancellationToken>>,
    wake: Notify,
    stop: CancellationToken,
    snapshot: watch::Sender<MonitorSnapshot>,
}

impl MonitorTask {
    pub fn new(session: String, collector: Arc<dyn Collector>) -> Arc<Self> {
        let (snapshot, _) = watch::channel(MonitorSnapshot {
            revision: 0,
            session_id: session,
            overview: None,
            error: false,
            refreshing: false,
            paused: false,
        });
        let task = Arc::new(Self {
            demands: Mutex::new(HashMap::new()),
            paused: Mutex::new(false),
            manual: Mutex::new(false),
            wake: Notify::new(),
            in_flight: Mutex::new(None),
            stop: CancellationToken::new(),
            snapshot,
        });
        let running = task.clone();
        tokio::spawn(async move {
            running.run(collector).await;
        });
        task
    }

    pub fn add(&self, id: String, token: String, cancellation: CancellationToken, seconds: u64) {
        self.demands.lock().unwrap().insert(
            id,
            Demand {
                token,
                cancellation,
                interval: Duration::from_secs(seconds.clamp(3, 3600)),
            },
        );
        self.wake.notify_one();
    }

    pub fn remove(&self, id: &str) {
        let mut demands = self.demands.lock().unwrap();
        demands.remove(id);
        if demands.is_empty() {
            self.stop.cancel();
        }
        self.wake.notify_one();
    }

    pub fn is_stopped(&self) -> bool {
        self.stop.is_cancelled()
    }
    pub fn cancel(&self) {
        self.stop.cancel();
    }
    pub fn snapshot(&self) -> MonitorSnapshot {
        self.snapshot.borrow().clone()
    }
    pub fn watch(&self) -> watch::Receiver<MonitorSnapshot> {
        self.snapshot.subscribe()
    }
    pub fn refresh(&self) {
        *self.paused.lock().unwrap() = false;
        *self.manual.lock().unwrap() = true;
        self.wake.notify_one();
    }
    pub fn pause(&self) {
        *self.paused.lock().unwrap() = true;
        *self.manual.lock().unwrap() = false;
        if let Some(cancellation) = self.in_flight.lock().unwrap().as_ref() {
            cancellation.cancel();
        }
        self.snapshot.send_modify(|s| {
            s.paused = true;
            s.refreshing = false;
            s.revision += 1;
        });
        self.wake.notify_one();
    }

    async fn run(self: Arc<Self>, collector: Arc<dyn Collector>) {
        let mut failures = 0;
        let mut started = false;
        let mut unavailable = false;
        let mut last_completion: Option<tokio::time::Instant> = None;
        loop {
            let demand = {
                let mut demands = self.demands.lock().unwrap();
                demands.retain(|_, d| !d.cancellation.is_cancelled());
                demands
                    .values()
                    .min_by_key(|d| d.interval)
                    .map(|d| (d.token.clone(), d.cancellation.clone(), d.interval))
            };
            let Some((token, cancellation, interval)) = demand else {
                if self.stop.is_cancelled() {
                    return;
                }
                tokio::select! { () = self.stop.cancelled() => return, () = self.wake.notified() => continue }
            };
            let paused = *self.paused.lock().unwrap();
            let manual = *self.manual.lock().unwrap();
            let next =
                last_completion.map_or_else(tokio::time::Instant::now, |last| last + interval);
            if (paused || unavailable) && !manual {
                tokio::select! {
                    () = self.stop.cancelled() => return,
                    () = cancellation.cancelled() => continue,
                    () = self.wake.notified() => continue,
                }
            }
            if !manual && tokio::time::Instant::now() < next {
                tokio::select! {
                    () = self.stop.cancelled() => return,
                    () = cancellation.cancelled() => continue,
                    () = self.wake.notified() => continue,
                    () = tokio::time::sleep_until(next) => {},
                }
            }
            // Pause can arrive while waiting; a wake must not issue another tick.
            if *self.paused.lock().unwrap() {
                continue;
            }
            *self.manual.lock().unwrap() = false;
            let resuming = self.snapshot.borrow().paused;
            let collection = CancellationToken::new();
            *self.in_flight.lock().unwrap() = Some(collection.clone());
            if *self.paused.lock().unwrap() {
                collection.cancel();
            }
            self.snapshot.send_modify(|s| {
                s.refreshing = true;
                s.paused = false;
                s.revision += 1;
            });
            let result = tokio::select! {
                biased;
                () = self.stop.cancelled() => return,
                () = collection.cancelled() => {
                    *self.in_flight.lock().unwrap() = None;
                    self.snapshot.send_modify(|s| { s.refreshing = false; s.paused = *self.paused.lock().unwrap(); s.revision += 1; });
                    collector.status("Monitor collection paused");
                    continue;
                },
                () = cancellation.cancelled() => { *self.in_flight.lock().unwrap() = None; last_completion = None; continue; },
                result = tokio::time::timeout(Duration::from_secs(45), collector.collect(&token)) =>
                    result.unwrap_or_else(|_| Err(invalid("Monitor collection timed out"))),
            }.and_then(validate_gpu);
            *self.in_flight.lock().unwrap() = None;
            if self.stop.is_cancelled() || cancellation.is_cancelled() || collection.is_cancelled()
            {
                continue;
            }
            if !started {
                collector.status("Monitor collection started");
                started = true;
            }
            match result {
                Ok(overview) => {
                    if failures > 0 || resuming {
                        collector.status("Monitor collection recovered");
                    }
                    failures = 0;
                    unavailable = !overview.available;
                    self.snapshot.send_modify(|s| {
                        s.overview = Some(overview);
                        s.error = false;
                        s.refreshing = false;
                        s.revision += 1;
                    });
                }
                Err(_) => {
                    if failures == 0 {
                        collector.status("Monitor collection failed");
                    }
                    failures += 1;
                    self.snapshot.send_modify(|s| {
                        if failures >= 3 {
                            s.overview = None;
                        }
                        s.error = true;
                        s.refreshing = false;
                        s.revision += 1;
                    });
                }
            }
            // Schedule after completion: no overlapping requests or catch-up bursts.
            last_completion = Some(tokio::time::Instant::now());
        }
    }
}

pub fn validate_gpu(value: Value) -> Result<RemoteGpuOverview> {
    if serde_json::to_vec(&value)?.len() > 1024 * 1024 {
        return Err(invalid("Monitor result exceeds 1 MiB"));
    }
    let overview: RemoteGpuOverview = serde_json::from_value(value)?;
    if overview.gpus.len() > 128
        || overview.processes.len() > 4096
        || overview.driver_version.len() > 256
        || overview.cuda_version.len() > 256
        || overview
            .gpus
            .iter()
            .any(|g| g.name.len() > 1024 || g.uuid.len() > 256 || g.pstate.len() > 128)
        || overview
            .processes
            .iter()
            .any(|p| p.process_name.len() > 4096 || p.gpu_uuid.len() > 256)
    {
        return Err(invalid("Monitor result exceeds GPU schema limits"));
    }
    Ok(overview)
}
