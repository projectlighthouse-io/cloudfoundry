use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::{Claimed, Error, JobId, NewJob, Store};

struct Row {
    id: JobId,
    job: Claimed,
    available_at: Instant,
    reserved_until: Option<Instant>,
}

/// In-process store, for tests and local experiments. Lost on restart.
#[derive(Default)]
pub struct MemoryStore {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    next: i64,
    jobs: Vec<Row>,
    failed: Vec<(Claimed, String)>,
}

impl MemoryStore {
    /// The state, even if a panicking job poisoned the lock: the data is
    /// still consistent, since every critical section is a few plain updates.
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Jobs not yet completed or failed for good.
    pub fn pending(&self) -> usize {
        self.lock().jobs.len()
    }

    /// Failed jobs as (name, error).
    pub fn failed(&self) -> Vec<(String, String)> {
        self.lock().failed.iter().map(|(j, e)| (j.name.clone(), e.clone())).collect()
    }
}

impl Store for MemoryStore {
    async fn push(&self, job: NewJob) -> Result<JobId, Error> {
        let mut s = self.lock();
        s.next += 1;
        let id = JobId(s.next);
        let job_row = Claimed { id, queue: job.queue, name: job.name, payload: job.payload, attempts: 0, max_attempts: job.max_attempts };
        s.jobs.push(Row { id, job: job_row, available_at: Instant::now() + job.delay, reserved_until: None });
        Ok(id)
    }

    async fn claim(&self, queues: &[String], limit: u32, visibility: Duration) -> Result<Vec<Claimed>, Error> {
        let now = Instant::now();
        let mut s = self.lock();
        let mut out = vec![];
        for row in s.jobs.iter_mut() {
            if out.len() as u32 == limit {
                break;
            }
            let free = row.reserved_until.is_none_or(|t| t < now);
            if free && row.available_at <= now && queues.contains(&row.job.queue) {
                row.reserved_until = Some(now + visibility);
                row.job.attempts += 1;
                out.push(row.job.clone());
            }
        }
        Ok(out)
    }

    async fn complete(&self, id: JobId) -> Result<(), Error> {
        self.lock().jobs.retain(|r| r.id != id);
        Ok(())
    }

    async fn fail(&self, id: JobId, error: &str, retry_in: Option<Duration>) -> Result<(), Error> {
        let mut s = self.lock();
        let Some(i) = s.jobs.iter().position(|r| r.id == id) else { return Ok(()) };
        if let Some(delay) = retry_in {
            let row = &mut s.jobs[i];
            row.reserved_until = None;
            row.available_at = Instant::now() + delay;
        } else {
            let row = s.jobs.remove(i);
            s.failed.push((row.job, error.to_owned()));
        }
        Ok(())
    }

    async fn extend(&self, id: JobId, visibility: Duration) -> Result<(), Error> {
        if let Some(row) = self.lock().jobs.iter_mut().find(|r| r.id == id) {
            row.reserved_until = Some(Instant::now() + visibility);
        }
        Ok(())
    }

    async fn retry_failed(&self, id: JobId) -> Result<Option<JobId>, Error> {
        let mut s = self.lock();
        let Some(i) = s.failed.iter().position(|(j, _)| j.id == id) else { return Ok(None) };
        let (mut job, _) = s.failed.remove(i);
        job.attempts = 0;
        s.jobs.push(Row { id, job, available_at: Instant::now(), reserved_until: None });
        Ok(Some(id))
    }
}
