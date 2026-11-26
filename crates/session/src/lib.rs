//! Load and save session data for a request.
//!
//! ```ignore
//! let sessions = session::layer(PostgresStore::new(pool), codec)
//!     .cookie_name("app_session")
//!     .lifetime(Duration::from_secs(14 * 86400));
//!
//! session.insert("cart", &cart)?;
//! let cart: Option<Cart> = session.get("cart")?;
//! session.flash("status", &"Saved")?;
//! session.regenerate();
//! ```
//!
//! Lazy: a request that never changes the session writes no row and sets no
//! cookie.

mod layer;
mod store;

#[cfg(feature = "command")]
pub mod command;

pub use layer::{SameSite, SessionLayer, layer};
#[cfg(feature = "mysql")]
pub use store::MysqlStore;
#[cfg(feature = "postgres")]
pub use store::PostgresStore;
#[cfg(feature = "sqlite")]
pub use store::SqliteStore;
pub use store::{MemoryStore, Prune, Record, Store};

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Key the CSRF token lives under. Cleared on `regenerate`.
const TOKEN_KEY: &str = "_token";

/// A random session id. Never derived from user data; only ever sent encrypted.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(String);

impl SessionId {
    fn random() -> Self {
        let mut b = [0u8; 32];
        getrandom::fill(&mut b).expect("OS random source");
        SessionId(URL_SAFE_NO_PAD.encode(b))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionId(..)")
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Stored {
    data: BTreeMap<String, Value>,
    /// Keys flashed by the request that saved this record.
    flash: Vec<String>,
}

#[derive(Default)]
struct State {
    id: Option<SessionId>,
    data: BTreeMap<String, Value>,
    flash_new: Vec<String>,
    flash_old: Vec<String>,
    expires_at: i64,
    dirty: bool,
    /// Ids to delete: replaced by `regenerate` or emptied.
    stale: Vec<SessionId>,
}

/// The current request's session. Cheap to clone; clones share state.
#[derive(Clone)]
pub struct Session(Arc<Mutex<State>>);

impl Session {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, serde_json::Error> {
        self.state().data.get(key).map(|v| T::deserialize(v)).transpose()
    }

    pub fn insert<T: Serialize + ?Sized>(&self, key: &str, value: &T) -> Result<(), serde_json::Error> {
        let value = serde_json::to_value(value)?;
        let mut s = self.state();
        s.data.insert(key.to_owned(), value);
        s.dirty = true;
        Ok(())
    }

    pub fn remove(&self, key: &str) {
        let mut s = self.state();
        if s.data.remove(key).is_some() {
            s.dirty = true;
        }
    }

    /// Store `value` for this request and the next one only.
    pub fn flash<T: Serialize + ?Sized>(&self, key: &str, value: &T) -> Result<(), serde_json::Error> {
        self.insert(key, value)?;
        let mut s = self.state();
        s.flash_old.retain(|k| k != key);
        if !s.flash_new.iter().any(|k| k == key) {
            s.flash_new.push(key.to_owned());
        }
        Ok(())
    }

    /// New id, same data. Call on login so a planted id is useless.
    /// Also drops the CSRF token, so it rotates with the id.
    pub fn regenerate(&self) {
        let mut s = self.state();
        if let Some(old) = s.id.take() {
            s.stale.push(old);
        }
        s.data.remove(TOKEN_KEY);
        s.dirty = true;
    }

    /// Drop all data and the id. Call on logout.
    pub fn invalidate(&self) {
        self.regenerate();
        let mut s = self.state();
        s.data.clear();
        s.flash_new.clear();
        s.flash_old.clear();
    }

    fn load(record: Record) -> Self {
        let stored: Stored = serde_json::from_slice(&record.data).unwrap_or_default();
        Session(Arc::new(Mutex::new(State {
            id: Some(record.id),
            data: stored.data,
            flash_old: stored.flash,
            expires_at: record.expires_at,
            ..State::default()
        })))
    }

    fn empty() -> Self {
        Session(Arc::new(Mutex::new(State::default())))
    }

    /// What to write once the handler is done.
    fn finish(&self, now: i64, lifetime: i64) -> Plan {
        let mut s = self.state();
        let old = std::mem::take(&mut s.flash_old);
        for k in old {
            if s.data.remove(&k).is_some() {
                s.dirty = true;
            }
        }
        // Sliding expiry without a write per request: refresh past half-life.
        let refresh = s.id.is_some() && s.expires_at - now < lifetime / 2;
        let mut plan = Plan { delete: std::mem::take(&mut s.stale), save: None, expire_cookie: false };

        if s.data.is_empty() {
            if s.dirty || refresh {
                if let Some(id) = s.id.take() {
                    plan.delete.push(id);
                }
            }
            plan.expire_cookie = !plan.delete.is_empty();
        } else if s.dirty || refresh {
            let id = s.id.get_or_insert_with(SessionId::random).clone();
            let stored = Stored { data: std::mem::take(&mut s.data), flash: std::mem::take(&mut s.flash_new) };
            let data = serde_json::to_vec(&stored).expect("session data is JSON");
            plan.save = Some(Record { id, data, expires_at: now + lifetime });
        }
        plan
    }
}

struct Plan {
    delete: Vec<SessionId>,
    save: Option<Record>,
    expire_cookie: bool,
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(feature = "route")]
mod route_impl {
    use std::{future::Future, sync::Arc};

    use route::{
        Body, FromRequest, FromRequestParts, IntoResponse,
        http::{Request, Response, StatusCode, request::Parts},
    };

    use crate::Session;

    impl<S: Send + Sync> FromRequestParts<S> for Session {
        async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
            parts.extensions.get::<Session>().cloned().ok_or_else(|| {
                tracing::error!("no session in request; add `session::layer` to the router");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            })
        }
    }

    impl<S: Send + Sync> FromRequest<S> for Session {
        fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
            route::from_parts(req, state)
        }
    }
}

#[cfg(feature = "csrf")]
impl csrf::TokenStore for Session {
    fn token(&self) -> Option<String> {
        self.get(TOKEN_KEY).ok().flatten()
    }

    fn set_token(&mut self, token: String) {
        self.insert(TOKEN_KEY, &token).expect("a string serializes");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86400;

    #[test]
    fn untouched_session_writes_nothing() {
        let plan = Session::empty().finish(0, DAY);
        assert!(plan.save.is_none() && plan.delete.is_empty() && !plan.expire_cookie);
    }

    #[test]
    fn flash_lives_one_request() {
        let s = Session::empty();
        s.insert("keep", &1).unwrap();
        s.flash("status", &"Saved").unwrap();
        let rec = s.finish(0, DAY).save.unwrap();

        let next = Session::load(rec);
        assert_eq!(next.get::<String>("status").unwrap().as_deref(), Some("Saved"));
        let rec = next.finish(0, DAY).save.expect("removing the flash is a change");

        let third = Session::load(rec);
        assert_eq!(third.get::<String>("status").unwrap(), None);
        assert_eq!(third.get::<i32>("keep").unwrap(), Some(1));
    }

    #[test]
    fn regenerate_and_invalidate() {
        let s = Session::empty();
        s.insert("user", &5).unwrap();
        s.insert(TOKEN_KEY, "t").unwrap();
        let first = s.finish(0, DAY).save.unwrap();

        let s = Session::load(first.clone());
        s.regenerate();
        let plan = s.finish(0, DAY);
        let second = plan.save.unwrap();
        assert!(plan.delete == vec![first.id.clone()] && second.id != first.id);
        assert_eq!(Session::load(second.clone()).get::<String>(TOKEN_KEY).unwrap(), None, "token rotates");

        let s = Session::load(second.clone());
        s.invalidate();
        let plan = s.finish(0, DAY);
        assert!(plan.save.is_none() && plan.delete == vec![second.id] && plan.expire_cookie);
    }

    #[test]
    fn refreshes_past_half_life_only() {
        let s = Session::empty();
        s.insert("k", &1).unwrap();
        let rec = s.finish(0, DAY).save.unwrap();
        assert!(Session::load(rec.clone()).finish(DAY / 4, DAY).save.is_none());
        assert!(Session::load(rec).finish(DAY * 3 / 4, DAY).save.is_some());
    }
}
