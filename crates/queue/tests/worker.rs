use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering::SeqCst},
    },
    time::Duration,
};

use queue::{Job, JobError, MemoryStore, NewJob, Queue, Store, Worker};
use serde::{Deserialize, Serialize};

#[derive(Default)]
struct Cx {
    ok: AtomicU32,
    flaky_calls: AtomicU32,
    slow_done: AtomicU32,
}

#[derive(Serialize, Deserialize)]
struct Ok1;
impl Job for Ok1 {
    const NAME: &'static str = "ok";
    type Context = Arc<Cx>;
    async fn run(self, cx: &Arc<Cx>) -> Result<(), JobError> {
        cx.ok.fetch_add(1, SeqCst);
        Ok(())
    }
}

/// Fails on its first run, succeeds on the second.
#[derive(Serialize, Deserialize)]
struct Flaky;
impl Job for Flaky {
    const NAME: &'static str = "flaky";
    type Context = Arc<Cx>;
    async fn run(self, cx: &Arc<Cx>) -> Result<(), JobError> {
        if cx.flaky_calls.fetch_add(1, SeqCst) == 0 { Err("first try fails".into()) } else { Ok(()) }
    }
    fn backoff(_: u32) -> Duration {
        Duration::ZERO
    }
}

#[derive(Serialize, Deserialize)]
struct Broken;
impl Job for Broken {
    const NAME: &'static str = "broken";
    const MAX_ATTEMPTS: u32 = 2;
    type Context = Arc<Cx>;
    async fn run(self, _: &Arc<Cx>) -> Result<(), JobError> {
        Err("always".into())
    }
    fn backoff(_: u32) -> Duration {
        Duration::ZERO
    }
}

#[derive(Serialize, Deserialize)]
struct Panics;
impl Job for Panics {
    const NAME: &'static str = "panics";
    const MAX_ATTEMPTS: u32 = 1;
    type Context = Arc<Cx>;
    async fn run(self, _: &Arc<Cx>) -> Result<(), JobError> {
        panic!("boom")
    }
}

#[derive(Serialize, Deserialize)]
struct Slow;
impl Job for Slow {
    const NAME: &'static str = "slow";
    type Context = Arc<Cx>;
    async fn run(self, cx: &Arc<Cx>) -> Result<(), JobError> {
        tokio::time::sleep(Duration::from_millis(200)).await;
        cx.slow_done.fetch_add(1, SeqCst);
        Ok(())
    }
}

fn worker(q: &Queue<MemoryStore>, cx: &Arc<Cx>) -> Worker<MemoryStore, Arc<Cx>> {
    Worker::new(q.clone(), cx.clone())
        .poll(Duration::from_millis(10))
        .register::<Ok1>()
        .register::<Flaky>()
        .register::<Broken>()
        .register::<Panics>()
        .register::<Slow>()
}

async fn until_idle(store: &MemoryStore) {
    while store.pending() > 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_retries_and_fails_jobs() {
    let q = Queue::new(MemoryStore::new());
    let cx = Arc::new(Cx::default());
    for _ in 0..3 {
        q.push(Ok1).await.unwrap();
    }
    q.push(Flaky).await.unwrap();
    q.push(Broken).await.unwrap();
    q.push(Panics).await.unwrap();
    q.store()
        .push(NewJob { queue: "default".into(), name: "ghost".into(), payload: "null".into(), max_attempts: 3, delay: Duration::ZERO })
        .await
        .unwrap();

    let store = q.clone();
    worker(&q, &cx).run(async move { until_idle(store.store()).await }).await;

    assert_eq!(cx.ok.load(SeqCst), 3);
    assert_eq!(cx.flaky_calls.load(SeqCst), 2, "retried once, then succeeded");
    let mut failed = q.store().failed();
    failed.sort();
    assert_eq!(failed.len(), 3, "{failed:?}");
    assert_eq!(failed[0], ("broken".into(), "always".into()));
    assert!(failed[1].0 == "ghost" && failed[1].1.contains("no handler"));
    assert!(failed[2].0 == "panics" && failed[2].1.contains("panicked"));
}

#[tokio::test(flavor = "multi_thread")]
async fn delayed_jobs_wait() {
    let q = Queue::new(MemoryStore::new());
    q.push_in(Duration::from_secs(60), Ok1).await.unwrap();
    let claimed = q.store().claim(&["default".into()], 10, Duration::from_secs(5)).await.unwrap();
    assert!(claimed.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_waits_for_running_jobs() {
    let q = Queue::new(MemoryStore::new());
    let cx = Arc::new(Cx::default());
    q.push(Slow).await.unwrap();

    let started = q.clone();
    // Stop as soon as the job is claimed (held, so no longer claimable).
    let shutdown = async move {
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if started.store().claim(&["default".into()], 1, Duration::from_secs(5)).await.unwrap().is_empty() {
                break;
            }
        }
    };
    worker(&q, &cx).run(shutdown).await;
    assert_eq!(cx.slow_done.load(SeqCst), 1, "running job finished before run() returned");
    assert_eq!(q.store().pending(), 0);
}
