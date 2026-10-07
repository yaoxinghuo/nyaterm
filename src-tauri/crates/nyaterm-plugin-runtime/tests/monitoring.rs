use async_trait::async_trait;
use nyaterm_plugin_runtime::{
    Error, Result,
    monitoring::{Collector, MonitorTask, validate_gpu},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct Fake {
    calls: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    delay: Duration,
    results: Mutex<VecDeque<Result<Value>>>,
}
impl Fake {
    fn new(results: Vec<Result<Value>>) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay: Duration::ZERO,
            results: Mutex::new(results.into()),
        })
    }
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
#[async_trait]
impl Collector for Fake {
    async fn collect(&self, scope: &str) -> Result<Value> {
        assert!(scope.starts_with("scope-"));
        self.calls.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        let _guard = Active(&self.active);
        tokio::time::sleep(self.delay).await;
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(overview(true)))
    }
}
fn overview(available: bool) -> Value {
    json!({"available":available,"driver_version":"550","cuda_version":"12","gpus":[],"processes":[]})
}
async fn settle() {
    for _ in 0..24 {
        tokio::task::yield_now().await;
    }
}
async fn advance(seconds: u64) {
    tokio::time::advance(Duration::from_secs(seconds)).await;
    settle().await;
}

#[tokio::test(start_paused = true)]
async fn shares_demands_stops_after_last_and_never_catches_up() {
    let fake = Arc::new(Fake {
        delay: Duration::from_secs(5),
        ..Arc::try_unwrap(Fake::new(vec![])).ok().unwrap()
    });
    let task = MonitorTask::new("ssh-1".into(), fake.clone());
    task.add("a".into(), "scope-a".into(), CancellationToken::new(), 3);
    task.add("b".into(), "scope-b".into(), CancellationToken::new(), 3);
    settle().await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    advance(4).await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    advance(1).await;
    assert!(task.snapshot().overview.is_some());
    task.remove("a");
    advance(3).await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    assert_eq!(fake.peak.load(Ordering::SeqCst), 1);
    task.remove("b");
    settle().await;
    assert_eq!(fake.active.load(Ordering::SeqCst), 0);
    advance(100).await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn unavailable_and_pause_require_explicit_refresh() {
    let fake = Fake::new(vec![Ok(overview(false)), Ok(overview(true))]);
    let task = MonitorTask::new("ssh-1".into(), fake.clone());
    task.add("a".into(), "scope-a".into(), CancellationToken::new(), 3);
    settle().await;
    advance(30).await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    task.refresh();
    settle().await;
    assert!(task.snapshot().overview.unwrap().available);
    task.pause();
    task.refresh();
    task.pause();
    advance(30).await;
    assert!(task.snapshot().paused);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    task.refresh();
    settle().await;
    assert!(!task.snapshot().paused);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 3);
    task.cancel();
}

#[tokio::test(start_paused = true)]
async fn failures_preserve_then_clear_and_can_recover() {
    let fail = || Err(Error::Runtime("failure".into()));
    let fake = Fake::new(vec![
        Ok(overview(true)),
        fail(),
        fail(),
        fail(),
        Ok(overview(true)),
    ]);
    let task = MonitorTask::new("ssh-1".into(), fake);
    task.add("a".into(), "scope-a".into(), CancellationToken::new(), 3);
    settle().await;
    let first = task.snapshot().revision;
    advance(3).await;
    assert!(task.snapshot().error && task.snapshot().overview.is_some());
    advance(3).await;
    assert!(task.snapshot().overview.is_some());
    advance(3).await;
    assert!(task.snapshot().overview.is_none());
    task.refresh();
    settle().await;
    assert!(!task.snapshot().error && task.snapshot().overview.is_some());
    assert!(task.snapshot().revision > first);
    task.cancel();
}

#[tokio::test(start_paused = true)]
async fn revocation_drops_inflight_and_rejects_late_publication() {
    let fake = Arc::new(Fake {
        delay: Duration::from_secs(5),
        ..Arc::try_unwrap(Fake::new(vec![])).ok().unwrap()
    });
    let cancellation = CancellationToken::new();
    let task = MonitorTask::new("ssh-1".into(), fake.clone());
    task.add("a".into(), "scope-a".into(), cancellation.clone(), 3);
    settle().await;
    cancellation.cancel();
    task.remove("a");
    settle().await;
    assert_eq!(fake.active.load(Ordering::SeqCst), 0);
    advance(30).await;
    assert!(task.snapshot().overview.is_none());
}

#[test]
fn rejects_untrusted_schema_and_excessive_results() {
    let mut value = overview(true);
    value["connectionId"] = json!("other");
    assert!(validate_gpu(value).is_err());
    let mut value = overview(true);
    value["processes"] = json!([{"gpu_uuid":"GPU-a","gpu_index":0,"pid":1,"process_name":"x".repeat(4097),"used_memory_mb":0}]);
    assert!(validate_gpu(value).is_err());
    assert!(validate_gpu(json!({"available":true})).is_err());
}

#[tokio::test(start_paused = true)]
async fn stopping_backend_cancels_collection_and_new_demands_remain_paused() {
    let fake = Arc::new(Fake {
        delay: Duration::from_secs(5),
        ..Arc::try_unwrap(Fake::new(vec![])).ok().unwrap()
    });
    let task = MonitorTask::new("ssh-1".into(), fake.clone());
    task.add("a".into(), "scope-a".into(), CancellationToken::new(), 3);
    settle().await;
    task.pause();
    settle().await;
    assert_eq!(fake.active.load(Ordering::SeqCst), 0);
    task.add("b".into(), "scope-b".into(), CancellationToken::new(), 3);
    advance(60).await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert!(task.snapshot().paused && task.snapshot().overview.is_none());
    task.refresh();
    settle().await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    advance(5).await;
    assert!(task.snapshot().overview.is_some());
    task.cancel();
}
