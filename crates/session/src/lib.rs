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
//!
//! Devices: a session that belongs to a user ([`Session::set_user`], which
//! `auth` calls on login) records its browser (user agent, IP) and when it was
//! last seen, refreshed at most once a minute. Stores that implement
//! [`Devices`] list a user's sessions and revoke them.

mod layer;
mod store;

#[cfg(feature = "command")]
pub mod command;

pub use layer::{Config, ConfigError, SameSite, SessionLayer, layer};
#[cfg(feature = "mysql")]
pub use store::MysqlStore;
#[cfg(feature = "postgres")]
pub use store::PostgresStore;
#[cfg(feature = "redis")]
pub use store::RedisStore;
#[cfg(feature = "sqlite")]
pub use store::SqliteStore;
pub use store::{Devices, MemoryStore, Prune, Record, Store};
/// The type of [`Device::public_id`], e.g. for `Path<session::Uuid>`.
pub use uuid::Uuid;

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
/// A user's session refreshes `last_seen_at` at most this often (seconds).
const SEEN_EVERY: i64 = 60;
/// Longer user agents are cut; this is plenty to recognise a browser.
const MAX_USER_AGENT: usize = 512;

/// The browser a session belongs to, for a "Browsers & devices" list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Device {
    /// Names the session in URLs and JSON; the session id itself never leaves the cookie.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_id: uuid::Uuid,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    /// Unix seconds.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub last_seen_at: i64,
}

/// A random session id. Never derived from user data; only ever sent encrypted.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(String);

impl SessionId {
    fn random() -> Self {
        let mut b = [0u8; 32];
        // No secure fallback exists: a guessable session id must not be issued.
        #[allow(clippy::expect_used, reason = "the OS random source failed; there is no safe fallback")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device: Option<Device>,
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
    user_id: Option<i64>,
    device: Option<Device>,
    /// This request's user agent and IP.
    seen: (Option<String>, Option<String>),
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
        s.user_id = None;
        s.device = None;
    }

    /// Record who the session belongs to (`auth` does on login), or `None`.
    /// Sessions with a user show up in [`Devices::devices`].
    pub fn set_user(&self, user: Option<i64>) {
        let mut s = self.state();
        s.user_id = user;
        if s.device.is_none() {
            s.device = Some(Device { public_id: uuid::Uuid::new_v4(), user_agent: None, ip: None, last_seen_at: 0 });
        }
        s.dirty = true;
    }

    pub fn user_id(&self) -> Option<i64> {
        self.state().user_id
    }

    /// This session's [`Device::public_id`], once it has one (after
    /// [`set_user`](Self::set_user) or its first save). Mark "this device" in
    /// a list with it, or keep it in [`Devices::revoke_others`].
    pub fn public_id(&self) -> Option<uuid::Uuid> {
        self.state().device.as_ref().map(|d| d.public_id)
    }

    /// The request's user agent and IP, set by the layer.
    fn seen(&self, user_agent: Option<String>, ip: Option<String>) {
        self.state().seen = (user_agent, ip);
    }

    fn load(record: Record) -> Self {
        let stored: Stored = serde_json::from_slice(&record.data).unwrap_or_default();
        Session(Arc::new(Mutex::new(State {
            id: Some(record.id),
            data: stored.data,
            flash_old: stored.flash,
            expires_at: record.expires_at,
            user_id: stored.user_id,
            device: stored.device,
            ..State::default()
        })))
    }

    fn empty() -> Self {
        Session(Arc::new(Mutex::new(State::default())))
    }

    /// An empty session outside any request or store, for tests of code that
    /// takes a `&Session`. Nothing it holds is saved.
    #[allow(clippy::new_without_default, reason = "a request's session comes from the layer; this one is spelled out")]
    pub fn new() -> Self {
        Self::empty()
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
        // A user's session also writes when the browser changed or a minute passed.
        // ponytail: the touch rewrites the whole record; a separate `touch` query if rows get big.
        let (ua, ip) = s.seen.clone();
        let touch = s.id.is_some()
            && s.user_id.is_some()
            && s.device.as_ref().is_none_or(|d| now - d.last_seen_at >= SEEN_EVERY || d.user_agent != ua || d.ip != ip);
        let refresh = refresh || touch;
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
            let device = s.device.get_or_insert_with(|| Device { public_id: uuid::Uuid::new_v4(), user_agent: None, ip: None, last_seen_at: 0 });
            (device.user_agent, device.ip, device.last_seen_at) = (ua, ip, now);
            let device = device.clone();
            let user_id = s.user_id;
            let stored = Stored { data: std::mem::take(&mut s.data), flash: std::mem::take(&mut s.flash_new), user_id, device: Some(device.clone()) };
            // Session values are already JSON, so this cannot fail; if it ever did, keep the old record.
            match serde_json::to_vec(&stored) {
                Ok(data) => plan.save = Some(Record { id, data, expires_at: now + lifetime, user_id, device: Some(device) }),
                Err(e) => tracing::error!("encoding the session failed: {e}"),
            }
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
mod extract {
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
        if let Err(e) = self.insert(TOKEN_KEY, &token) {
            tracing::error!("storing the CSRF token failed: {e}");
        }
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
    fn a_users_session_records_its_device_once_a_minute() {
        let s = Session::empty();
        s.seen(Some("Firefox".into()), Some("10.0.0.1".into()));
        s.insert("_auth_id", &7).unwrap();
        s.set_user(Some(7));
        let public_id = s.public_id().unwrap();
        let rec = s.finish(1000, DAY).save.unwrap();
        let device = rec.device.clone().unwrap();
        assert_eq!((rec.user_id, device.public_id, device.user_agent.as_deref(), device.last_seen_at), (Some(7), public_id, Some("Firefox"), 1000));

        let seen = |rec: &Record, at: i64, ua: &str| {
            let s = Session::load(rec.clone());
            s.seen(Some(ua.into()), Some("10.0.0.1".into()));
            s.finish(at, DAY).save
        };
        assert!(seen(&rec, 1059, "Firefox").is_none(), "within the minute: no write");
        let later = seen(&rec, 1060, "Firefox").unwrap();
        let device = later.device.clone().unwrap();
        assert_eq!((device.public_id, device.last_seen_at), (public_id, 1060));
        assert!(seen(&rec, 1001, "Safari").is_some(), "another browser writes at once");

        let s = Session::load(later);
        s.regenerate();
        assert_eq!(s.public_id(), Some(public_id), "a new id keeps the device");
        s.invalidate();
        assert_eq!((s.public_id(), s.user_id()), (None, None), "logout forgets it");

        let anon = Session::empty();
        anon.insert("k", &1).unwrap();
        let rec = anon.finish(0, DAY).save.unwrap();
        assert!(Session::load(rec).finish(120, DAY).save.is_none(), "anonymous sessions are not touched");
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
