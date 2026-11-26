//! Persist, claim, run and retry jobs.
//!
//! ```ignore
//! #[derive(Serialize, Deserialize)]
//! struct ExecuteRun { run_id: i64 }
//!
//! impl Job for ExecuteRun {
//!     const NAME: &'static str = "execute_run";
//!     type Context = App;
//!
//!     async fn run(self, app: &App) -> Result<(), JobError> { /* ... */ Ok(()) }
//! }
//!
//! queue.push(ExecuteRun { run_id }).await?;
//!
//! Worker::new(queue, app).register::<ExecuteRun>().concurrency(8).run(shutdown).await;
//! ```
//!
//! Delivery is at-least-once: a job can run again if a worker dies mid-job, so
//! jobs must be idempotent. Modeled on Laravel's database queue driver.

mod memory;
mod worker;

#[cfg(feature = "command")]
pub mod command;
#[cfg(feature = "postgres")]
mod postgres;

pub use memory::MemoryStore;
#[cfg(feature = "postgres")]
pub use postgres::PostgresStore;
pub use worker::{BoxFuture, Worker, shutdown_signal};

use std::{fmt, future::Future, sync::Arc, time::Duration};

use serde::{Serialize, de::DeserializeOwned};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
/// What a job returns on failure. Any error converts.
pub type JobError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub i64);

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A unit of background work. The payload is the struct itself, as JSON.
pub trait Job: Serialize + DeserializeOwned + Send + 'static {
    /// Stored with the payload. Keep it stable: renaming the Rust type is fine,
    /// changing this strands queued jobs.
    const NAME: &'static str;
    const QUEUE: &'static str = "default";
    /// Total runs before the job moves to failed jobs.
    const MAX_ATTEMPTS: u32 = 3;

    type Context: Send + Sync + 'static;

    fn run(self, cx: &Self::Context) -> impl Future<Output = Result<(), JobError>> + Send;

    /// Wait before retry number `attempt` (1 after the first failure).
    /// Default: 10s, 20s, 40s, ... capped at one hour.
    fn backoff(attempt: u32) -> Duration {
        default_backoff(attempt)
    }
}

/// 10s, 20s, 40s, ... capped at one hour.
pub fn default_backoff(attempt: u32) -> Duration {
    Duration::from_secs(10u64.saturating_mul(1 << attempt.saturating_sub(1).min(16)).min(3600))
}

/// A job to insert.
pub struct NewJob {
    pub queue: String,
    pub name: String,
    pub payload: String,
    pub max_attempts: u32,
    pub delay: Duration,
}

/// A job a worker now holds until `visibility` runs out.
#[derive(Clone, Debug)]
pub struct Claimed {
    pub id: JobId,
    pub queue: String,
    pub name: String,
    pub payload: String,
    /// Including this run.
    pub attempts: u32,
    pub max_attempts: u32,
}

/// Where jobs live. Times are the store's own clock, so app servers with
/// skewed clocks agree.
pub trait Store: Send + Sync + 'static {
    fn push(&self, job: NewJob) -> impl Future<Output = Result<JobId, Error>> + Send;
    /// Up to `limit` jobs that are due and not held, oldest first. Each is held
    /// for `visibility`; if not completed or failed by then, it is claimable again.
    fn claim(&self, queues: &[String], limit: u32, visibility: Duration) -> impl Future<Output = Result<Vec<Claimed>, Error>> + Send;
    fn complete(&self, id: JobId) -> impl Future<Output = Result<(), Error>> + Send;
    /// `Some(delay)`: release for another attempt after `delay`.
    /// `None`: move to failed jobs.
    fn fail(&self, id: JobId, error: &str, retry_in: Option<Duration>) -> impl Future<Output = Result<(), Error>> + Send;
    /// Move a failed job back to its queue with fresh attempts. `None` if no such failed job.
    fn retry_failed(&self, id: JobId) -> impl Future<Output = Result<Option<JobId>, Error>> + Send;
}

/// Push jobs. Cheap to clone.
pub struct Queue<St> {
    store: Arc<St>,
}

impl<St> Clone for Queue<St> {
    fn clone(&self) -> Self {
        Self { store: self.store.clone() }
    }
}

impl<St: Store> Queue<St> {
    pub fn new(store: St) -> Self {
        Self { store: Arc::new(store) }
    }

    pub fn store(&self) -> &St {
        &self.store
    }

    pub async fn push<J: Job>(&self, job: J) -> Result<JobId, Error> {
        self.push_in(Duration::ZERO, job).await
    }

    /// Run no earlier than `delay` from now.
    pub async fn push_in<J: Job>(&self, delay: Duration, job: J) -> Result<JobId, Error> {
        self.store.push(new_job(&job, delay)?).await
    }
}

pub(crate) fn new_job<J: Job>(job: &J, delay: Duration) -> Result<NewJob, Error> {
    Ok(NewJob {
        queue: J::QUEUE.to_owned(),
        name: J::NAME.to_owned(),
        payload: serde_json::to_string(job)?,
        max_attempts: J::MAX_ATTEMPTS,
        delay,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct J;
    impl Job for J {
        const NAME: &'static str = "j";
        type Context = ();
        async fn run(self, _: &()) -> Result<(), JobError> {
            Ok(())
        }
    }

    #[test]
    fn default_backoff() {
        let secs: Vec<u64> = [1, 2, 3, 9, 40].map(|a| J::backoff(a).as_secs()).to_vec();
        assert_eq!(secs, [10, 20, 40, 2560, 3600]);
    }
}
