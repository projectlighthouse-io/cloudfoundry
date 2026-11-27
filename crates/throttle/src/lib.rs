//! Rate limiting for anything: requests, logins, calls to an external API,
//! or a function you don't want to run too often.
//!
//! A [`Limiter`] pairs an [`Algorithm`] (how hits are counted) with a
//! [`Store`] (where the counts live), and keeps a separate limit per key.
//!
//! ```
//! # async fn run() -> Result<(), throttle::Error> {
//! use std::time::Duration;
//! use throttle::{FixedWindow, Limiter, TokenBucket};
//!
//! // 5 per minute per key.
//! let logins = Limiter::memory(FixedWindow::new(5, Duration::from_secs(60)));
//! let d = logins.hit("ada@example.com").await?;
//! if !d.allowed {
//!     println!("try again in {:?}", d.retry_after);
//! }
//!
//! // Bursts of 10, refilled at 1 per second: wait for a slot, then call.
//! let github = Limiter::memory(TokenBucket::new(10, 1, Duration::from_secs(1)));
//! github.wait("api").await?;
//! # Ok(()) }
//! ```

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

mod algorithms;
pub use algorithms::{FixedWindow, SlidingWindow, TokenBucket};
#[cfg(feature = "command")]
pub mod command;
#[cfg(any(feature = "postgres", feature = "sqlite", feature = "mysql"))]
mod sql;
#[cfg(feature = "mysql")]
pub use sql::MysqlStore;
#[cfg(feature = "postgres")]
pub use sql::PostgresStore;
#[cfg(feature = "sqlite")]
pub use sql::SqliteStore;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// The answer to one hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,
    /// Hits left after this one.
    pub remaining: u64,
    /// When a denied hit could succeed. Zero when allowed.
    pub retry_after: Duration,
}

/// How hits are counted. `state` is whatever the algorithm stored last time
/// for this key (`None` for a new or expired key); times are unix milliseconds.
pub trait Algorithm: Send + Sync + 'static {
    /// Decide on a hit costing `cost`. Returns the decision and, when allowed,
    /// the new state and when it may be forgotten (unix ms). Denied hits
    /// change nothing.
    fn decide(&self, state: Option<&[u8]>, now: i64, cost: u64) -> (Decision, Option<(Vec<u8>, i64)>);
}

/// Where each key's state lives. Times are unix milliseconds.
pub trait Store: Send + Sync + 'static {
    /// The state for `key`, unless it has expired.
    fn get(&self, key: &str, now: i64) -> impl Future<Output = Result<Option<Vec<u8>>, Error>> + Send;
    /// Read-modify-write, atomic per key: `f` gets the unexpired state and
    /// returns the new one with its expiry (or `None` to leave it).
    fn update<F, R>(&self, key: &str, now: i64, f: F) -> impl Future<Output = Result<R, Error>> + Send
    where
        F: FnOnce(Option<&[u8]>) -> (R, Option<(Vec<u8>, i64)>) + Send,
        R: Send;
    fn clear(&self, key: &str) -> impl Future<Output = Result<(), Error>> + Send;
    /// Delete expired state; returns how many keys.
    fn prune(&self, now: i64) -> impl Future<Output = Result<u64, Error>> + Send;
}

/// In-process store: limits are per process and lost on restart. Use a SQL
/// store when several servers must share a limit.
#[derive(Default)]
pub struct MemoryStore(Mutex<Memory>);

#[derive(Default)]
struct Memory {
    map: HashMap<String, (Vec<u8>, i64)>,
    writes: u32,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Store for MemoryStore {
    async fn get(&self, key: &str, now: i64) -> Result<Option<Vec<u8>>, Error> {
        Ok(self.0.lock().unwrap().map.get(key).filter(|(_, exp)| *exp > now).map(|(s, _)| s.clone()))
    }

    async fn update<F, R>(&self, key: &str, now: i64, f: F) -> Result<R, Error>
    where
        F: FnOnce(Option<&[u8]>) -> (R, Option<(Vec<u8>, i64)>) + Send,
        R: Send,
    {
        let mut m = self.0.lock().unwrap();
        let current = m.map.get(key).filter(|(_, exp)| *exp > now).map(|(s, _)| s.as_slice());
        let (out, new) = f(current);
        if let Some(entry) = new {
            m.map.insert(key.to_owned(), entry);
            // ponytail: sweep expired keys every 1024 writes so the map stays bounded by live keys.
            m.writes = m.writes.wrapping_add(1);
            if m.writes % 1024 == 0 {
                m.map.retain(|_, (_, exp)| *exp > now);
            }
        }
        Ok(out)
    }

    async fn clear(&self, key: &str) -> Result<(), Error> {
        self.0.lock().unwrap().map.remove(key);
        Ok(())
    }

    async fn prune(&self, now: i64) -> Result<u64, Error> {
        let mut m = self.0.lock().unwrap();
        let before = m.map.len();
        m.map.retain(|_, (_, exp)| *exp > now);
        Ok((before - m.map.len()) as u64)
    }
}

/// A rate limit, kept separately per key.
pub struct Limiter<A, St = MemoryStore> {
    algorithm: Arc<A>,
    store: Arc<St>,
}

impl<A, St> Clone for Limiter<A, St> {
    fn clone(&self) -> Self {
        Self { algorithm: self.algorithm.clone(), store: self.store.clone() }
    }
}

impl<A: Algorithm> Limiter<A> {
    /// A limiter with its own [`MemoryStore`].
    pub fn memory(algorithm: A) -> Self {
        Self::new(MemoryStore::new(), algorithm)
    }
}

impl<A: Algorithm, St: Store> Limiter<A, St> {
    pub fn new(store: St, algorithm: A) -> Self {
        Self { algorithm: Arc::new(algorithm), store: Arc::new(store) }
    }

    pub fn store(&self) -> &St {
        &self.store
    }

    /// Count one hit. Denied hits are not counted.
    pub async fn hit(&self, key: &str) -> Result<Decision, Error> {
        self.hit_n(key, 1).await
    }

    /// Count a hit that costs `cost`, e.g. a batch call worth several requests.
    pub async fn hit_n(&self, key: &str, cost: u64) -> Result<Decision, Error> {
        let now = unix_ms();
        self.store.update(key, now, |state| self.algorithm.decide(state, now, cost)).await
    }

    /// What a hit would get, without counting it. For "refuse before doing
    /// the work, count only failures" (login).
    pub async fn check(&self, key: &str) -> Result<Decision, Error> {
        let now = unix_ms();
        let state = self.store.get(key, now).await?;
        Ok(self.algorithm.decide(state.as_deref(), now, 1).0)
    }

    /// Forget `key`'s hits.
    pub async fn clear(&self, key: &str) -> Result<(), Error> {
        self.store.clear(key).await
    }

    /// Wait until a hit is allowed, then count it. For outgoing calls:
    /// `limiter.wait("github").await?; client.get(..)`.
    pub async fn wait(&self, key: &str) -> Result<(), Error> {
        loop {
            let d = self.hit(key).await?;
            if d.allowed {
                return Ok(());
            }
            tokio::time::sleep(d.retry_after).await;
        }
    }
}

/// Now, in the unix milliseconds stores and algorithms use.
pub fn unix_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("clock after 1970").as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn check_does_not_count_and_clear_resets() {
        let l = Limiter::memory(FixedWindow::new(2, Duration::from_secs(60)));
        assert!(l.check("k").await.unwrap().allowed);
        assert!(l.check("k").await.unwrap().allowed);
        assert_eq!(l.hit("k").await.unwrap().remaining, 1);
        assert_eq!(l.hit("k").await.unwrap().remaining, 0);
        let denied = l.hit("k").await.unwrap();
        assert!(!denied.allowed);
        assert!(denied.retry_after > Duration::from_secs(59));
        assert!(!l.check("k").await.unwrap().allowed);
        assert!(l.hit("other").await.unwrap().allowed, "keys are separate");
        l.clear("k").await.unwrap();
        assert!(l.hit("k").await.unwrap().allowed);
    }

    #[tokio::test]
    async fn wait_sleeps_until_allowed() {
        let l = Limiter::memory(TokenBucket::new(1, 1, Duration::from_millis(50)));
        let start = std::time::Instant::now();
        l.wait("k").await.unwrap();
        l.wait("k").await.unwrap();
        assert!(start.elapsed() >= Duration::from_millis(40), "{:?}", start.elapsed());
    }

    #[tokio::test]
    async fn memory_prunes_expired() {
        let s = MemoryStore::new();
        s.update("a", 0, |_| ((), Some((vec![1], 10)))).await.unwrap();
        s.update("b", 0, |_| ((), Some((vec![2], 100)))).await.unwrap();
        assert_eq!(s.get("a", 10).await.unwrap(), None, "expired");
        assert_eq!(s.prune(50).await.unwrap(), 1);
        assert_eq!(s.len(), 1);
    }
}
