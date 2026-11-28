use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use serde::de::DeserializeOwned;
use tokio::task::JoinSet;

use crate::{Claimed, Job, JobError, Queue, Store};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type Run<C> = Arc<dyn Fn(Arc<C>, String) -> BoxFuture<Result<(), JobError>> + Send + Sync>;

#[derive(Clone)]
struct Handler<C> {
    run: Run<C>,
    backoff: fn(u32) -> Duration,
}

/// Worker settings. Deserializable with feature `configured`; missing keys keep
/// their defaults, unknown keys are an error:
///
/// ```toml
/// [queue]
/// queues = ["high", "default"]
/// concurrency = 8
/// visibility = "10m"
/// poll = "500ms"
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "configured",
    derive(configured::serde::Deserialize, configured::serde::Serialize),
    serde(crate = "configured::serde", default, deny_unknown_fields)
)]
pub struct WorkerConfig {
    pub queues: Vec<String>,
    /// Jobs run at once.
    pub concurrency: usize,
    /// How long a claimed job is held without a heartbeat. SQL stores keep
    /// whole seconds, so it must be at least one second.
    #[cfg_attr(feature = "configured", serde(with = "configured::duration"))]
    pub visibility: Duration,
    /// Idle wait between claims when the queues are empty.
    #[cfg_attr(feature = "configured", serde(with = "configured::duration"))]
    pub poll: Duration,
}

/// Queue `default`, 4 at a time, 5 minute visibility, poll every second.
impl Default for WorkerConfig {
    fn default() -> Self {
        Self { queues: vec!["default".into()], concurrency: 4, visibility: Duration::from_secs(300), poll: Duration::from_secs(1) }
    }
}

/// A [`WorkerConfig`] that would run no jobs, run jobs twice, or spin.
#[derive(Debug)]
pub struct ConfigError(String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "queue config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl WorkerConfig {
    /// Run by [`Worker::config`].
    pub fn check(&self) -> Result<(), ConfigError> {
        let err = |m: &str| Err(ConfigError(m.into()));
        if self.queues.is_empty() || self.queues.iter().any(String::is_empty) {
            return err("queues must name at least one queue, none empty");
        }
        if self.concurrency == 0 {
            return err("concurrency must be at least 1");
        }
        // Below a second, SQL stores would store 0 and hand running jobs to other workers.
        if self.visibility < Duration::from_secs(1) {
            return err("visibility must be at least 1s");
        }
        if self.poll.is_zero() {
            return err("poll must be above zero; 0 busy-loops on an empty queue");
        }
        Ok(())
    }
}

/// Claims and runs jobs. Built at startup with every job it can receive.
pub struct Worker<St, C> {
    queue: Queue<St>,
    cx: Arc<C>,
    handlers: HashMap<String, Handler<C>>,
    queues: Vec<String>,
    concurrency: usize,
    visibility: Duration,
    poll: Duration,
}

impl<St: Store, C: Send + Sync + 'static> Worker<St, C> {
    /// With [`WorkerConfig::default`].
    pub fn new(queue: Queue<St>, cx: C) -> Self {
        let WorkerConfig { queues, concurrency, visibility, poll } = WorkerConfig::default();
        Self { queue, cx: Arc::new(cx), handlers: HashMap::new(), queues, concurrency, visibility, poll }
    }

    /// Replace all settings after [`WorkerConfig::check`]. Setters called
    /// after this change single fields.
    pub fn config(mut self, cfg: WorkerConfig) -> Result<Self, ConfigError> {
        cfg.check()?;
        let WorkerConfig { queues, concurrency, visibility, poll } = cfg;
        (self.queues, self.concurrency, self.visibility, self.poll) = (queues, concurrency, visibility, poll);
        Ok(self)
    }

    pub fn register<J: Job<Context = C>>(self) -> Self {
        self.handle(J::NAME, J::backoff, |cx: Arc<C>, job: J| async move { job.run(&cx).await })
    }

    /// Run payloads stored under `name` with `f`. For crates that turn their
    /// own things into jobs, such as queued event listeners.
    pub fn handle<T, F, Fut>(self, name: &str, backoff: fn(u32) -> Duration, f: F) -> Self
    where
        T: DeserializeOwned + Send + 'static,
        F: Fn(Arc<C>, T) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), JobError>> + Send + 'static,
    {
        let f = Arc::new(f);
        self.handle_raw(name, backoff, move |cx, payload| {
            let f = f.clone();
            Box::pin(async move {
                let value: T = serde_json::from_str(&payload).map_err(|e| format!("payload does not decode: {e}"))?;
                f(cx, value).await
            })
        })
    }

    /// `handle` with the JSON payload undecoded.
    pub fn handle_raw(
        mut self,
        name: &str,
        backoff: fn(u32) -> Duration,
        run: impl Fn(Arc<C>, String) -> BoxFuture<Result<(), JobError>> + Send + Sync + 'static,
    ) -> Self {
        let handler = Handler { run: Arc::new(run), backoff };
        assert!(self.handlers.insert(name.to_owned(), handler).is_none(), "job `{name}` registered twice");
        self
    }

    pub fn queues<I: IntoIterator<Item = S>, S: Into<String>>(mut self, queues: I) -> Self {
        self.queues = queues.into_iter().map(Into::into).collect();
        self
    }

    pub fn concurrency(mut self, n: usize) -> Self {
        self.concurrency = n.max(1);
        self
    }

    /// How long a claimed job is held without a heartbeat. The worker extends
    /// it every third of this while the job runs, so it bounds how long a job
    /// from a dead worker waits, not how long a job may run.
    pub fn visibility(mut self, d: Duration) -> Self {
        self.visibility = d;
        self
    }

    /// Idle wait between claims when the queues are empty.
    pub fn poll(mut self, d: Duration) -> Self {
        self.poll = d;
        self
    }

    /// Run until `shutdown` resolves, then stop claiming and wait for running
    /// jobs to finish.
    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        let shared = Arc::new(Shared { queue: self.queue, cx: self.cx, handlers: self.handlers, visibility: self.visibility });
        let mut running: JoinSet<()> = JoinSet::new();
        tokio::pin!(shutdown);

        loop {
            let free = self.concurrency.saturating_sub(running.len());
            let claimed = if free == 0 {
                vec![]
            } else {
                match shared.queue.store.claim(&self.queues, free as u32, self.visibility).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("claiming jobs failed: {e}");
                        vec![]
                    }
                }
            };
            let full = claimed.len() == free;
            for job in claimed {
                running.spawn(shared.clone().execute(job));
            }
            if full && free > 0 {
                continue; // more may be waiting
            }

            tokio::select! {
                _ = &mut shutdown => break,
                Some(done) = running.join_next(), if !running.is_empty() => log_join(done),
                _ = tokio::time::sleep(self.poll) => {}
            }
        }

        tracing::info!("worker stopping; waiting for {} running jobs", running.len());
        while let Some(done) = running.join_next().await {
            log_join(done);
        }
    }
}

fn log_join(done: Result<(), tokio::task::JoinError>) {
    if let Err(e) = done {
        tracing::error!("job task failed: {e}");
    }
}

struct Shared<St, C> {
    queue: Queue<St>,
    cx: Arc<C>,
    handlers: HashMap<String, Handler<C>>,
    visibility: Duration,
}

impl<St: Store, C: Send + Sync + 'static> Shared<St, C> {
    async fn execute(self: Arc<Self>, job: Claimed) {
        let store = &self.queue.store;
        let Some(h) = self.handlers.get(&job.name) else {
            let msg = format!("no handler for job `{}`; register it on the worker", job.name);
            return record(store.fail(job.id, &msg, None).await, &job);
        };
        if job.attempts > job.max_attempts {
            // Claimed again after its hold ran out: the worker died or the job outran `visibility`.
            let msg = "attempted too many times; a worker may have died, or the job outlived the visibility timeout";
            return record(store.fail(job.id, msg, None).await, &job);
        }

        // Run in its own task so a panic is caught and recorded like an error.
        let mut task = tokio::spawn((h.run)(self.cx.clone(), job.payload.clone()));
        // Heartbeat: extend the hold every third of it while the job runs.
        let every = (self.visibility / 3).max(Duration::from_millis(10));
        let mut beat = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        let outcome = loop {
            tokio::select! {
                done = &mut task => break done,
                _ = beat.tick() => {
                    if let Err(e) = store.extend(job.id, self.visibility).await {
                        tracing::warn!(job = %job.id, name = %job.name, "extending job hold failed: {e}");
                    }
                }
            }
        };
        let error = match outcome {
            Ok(Ok(())) => return record(store.complete(job.id).await, &job),
            Ok(Err(e)) => e.to_string(),
            Err(e) => format!("job panicked: {e}"),
        };

        if job.attempts < job.max_attempts {
            let delay = (h.backoff)(job.attempts);
            tracing::warn!(job = %job.id, name = %job.name, attempt = job.attempts, "job failed, retrying in {delay:?}: {error}");
            record(store.fail(job.id, &error, Some(delay)).await, &job);
        } else {
            tracing::error!(job = %job.id, name = %job.name, attempt = job.attempts, "job failed for good: {error}");
            record(store.fail(job.id, &error, None).await, &job);
        }
    }
}

fn record(result: Result<(), crate::Error>, job: &Claimed) {
    if let Err(e) = result {
        // The job stays held and becomes claimable again after `visibility`.
        tracing::error!(job = %job.id, name = %job.name, "recording job result failed: {e}");
    }
}

/// Resolves on Ctrl-C or, on Unix, SIGTERM: what process managers send to stop.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
