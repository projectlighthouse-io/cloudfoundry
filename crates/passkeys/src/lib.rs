//! Passkeys (WebAuthn, on `webauthn-rs`): passwordless sign-in with
//! discoverable credentials, and a passkey as the second factor of a
//! password login.
//!
//! Every ceremony is two requests. `start_*` returns options for the browser
//! (`navigator.credentials.create/get`) and keeps the ceremony state in the
//! session for five minutes; `finish_*` checks the browser's answer against
//! it. A state is used once, whatever the outcome.
//!
//! ```ignore
//! let pk = Passkeys::new(store, "https://acme.test", "Acme")?;
//! let options = pk.start_registration(&session, user.id, &user.email, &user.name).await?; // to the browser
//! let added = pk.finish_registration(&session, user.id, "MacBook", &answer).await?;
//!
//! let options = pk.start_login(&session)?;                  // no username: the browser picks the passkey
//! let used = pk.finish_login(&session, &answer).await?;     // then auth.login(user used.user_id)
//! ```
//!
//! [`Passkeys`] does not log anyone in, and does not check sudo mode: the
//! published handlers call `auth.login` and take `auth::RecentlyConfirmed`.
//!
//! Ceremony state is serialised into the session. The session must be stored
//! server side (memory, SQL or Redis store): a client-side session would let
//! a browser replay an old challenge.

use std::{
    fmt,
    future::Future,
    net::IpAddr,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use session::Session;
use webauthn_rs::prelude::{
    DiscoverableAuthentication, PasskeyAuthentication, PasskeyRegistration, Url, Webauthn, WebauthnBuilder,
};
use webauthn_rs_proto::ResidentKeyRequirement;

pub use db::Uuid;
pub use store::{MemoryStore, NewPasskey, Passkey, Store};
#[cfg(feature = "mysql")]
pub use sql::MysqlStore;
#[cfg(feature = "postgres")]
pub use sql::PostgresStore;
#[cfg(feature = "sqlite")]
pub use sql::SqliteStore;
/// What the browser sends and receives, and the stored credential.
pub use webauthn_rs::prelude::{
    AuthenticationResult, CreationChallengeResponse, Passkey as Credential, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse, WebauthnError,
};
/// The underlying library, for settings [`Passkeys::new`] does not cover.
pub use webauthn_rs;

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]
mod sql;
mod store;

const REGISTER: &str = "_passkeys_register";
const SECOND_FACTOR: &str = "_passkeys_second_factor";
const LOGIN: &str = "_passkeys_login";
/// How long a ceremony waits for the browser: webauthn-rs' default authenticator timeout.
const CEREMONY_SECS: i64 = 300;

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug)]
pub enum Error {
    /// No ceremony was started in this session, it expired, it was already
    /// used, or it was started for another user: start again.
    NoCeremony,
    /// A second factor for a user without passkeys.
    NoPasskeys,
    /// The browser's answer did not verify (wrong challenge, origin, signature, ...).
    Rejected(WebauthnError),
    /// This credential is already registered (to this or another account).
    AlreadyRegistered,
    /// The answer names a credential that is not stored, or not this user's.
    UnknownCredential,
    /// Too many failed sign-ins or second factors from this IP; try again after this long.
    Throttled(Duration),
    /// The store, the session or the configuration failed.
    Other(BoxError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCeremony => f.write_str("no passkey ceremony is waiting in this session"),
            Self::NoPasskeys => f.write_str("the user has no passkeys"),
            Self::Rejected(e) => write!(f, "the passkey was rejected: {e}"),
            Self::AlreadyRegistered => f.write_str("this passkey is already registered"),
            Self::UnknownCredential => f.write_str("unknown passkey"),
            Self::Throttled(d) => write!(f, "too many attempts; try again in {} seconds", d.as_secs().max(1)),
            Self::Other(e) => write!(f, "passkeys: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// Wrap a store or library error.
    pub fn other(e: impl Into<BoxError>) -> Self {
        Self::Other(e.into())
    }
}

/// Events [`Passkeys`] emits with feature `event` (`Passkeys::events`).
/// Without it they are plain structs for the app to emit.
pub mod events {
    use serde::{Deserialize, Serialize};

    use crate::{Passkey, Uuid};

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct PasskeyAdded {
        pub user_id: i64,
        pub passkey: Uuid<Passkey>,
        pub name: String,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct PasskeyRemoved {
        pub user_id: i64,
        pub passkey: Uuid<Passkey>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct PasskeyUsed {
        pub user_id: i64,
        pub passkey: Uuid<Passkey>,
        /// `true`: signed in with the passkey alone; `false`: as the second factor.
        pub passwordless: bool,
    }
}

#[cfg_attr(not(feature = "event"), allow(dead_code))]
enum Emitted {
    Added(events::PasskeyAdded),
    Removed(events::PasskeyRemoved),
    Used(events::PasskeyUsed),
}

#[cfg(feature = "event")]
type Emit = Arc<dyn Fn(Emitted) -> BoxFuture<'static, Result<(), BoxError>> + Send + Sync>;

/// Failure limiter with its algorithm and store erased.
trait Limiter: Send + Sync {
    fn check<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Duration>, BoxError>>;
    fn hit<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), BoxError>>;
    fn clear<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), BoxError>>;
}

#[cfg(feature = "throttle")]
impl<A: throttle::Algorithm, St: throttle::Store> Limiter for throttle::Limiter<A, St> {
    fn check<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Duration>, BoxError>> {
        Box::pin(async move { Ok(Some(throttle::Limiter::check(self, key).await?).filter(|d| !d.allowed).map(|d| d.retry_after)) })
    }
    fn hit<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), BoxError>> {
        Box::pin(async move { throttle::Limiter::hit(self, key).await.map(drop) })
    }
    fn clear<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), BoxError>> {
        Box::pin(throttle::Limiter::clear(self, key))
    }
}

/// A ceremony waiting in the session.
#[derive(Serialize, Deserialize)]
struct Ceremony<T> {
    state: T,
    expires_at: i64,
}

/// The rules over a [`Store`]. Cheap to clone.
pub struct Passkeys<S> {
    store: Arc<S>,
    webauthn: Arc<Webauthn>,
    limiter: Option<Arc<dyn Limiter>>,
    #[cfg(feature = "event")]
    emit: Option<Emit>,
}

impl<S> Clone for Passkeys<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            webauthn: self.webauthn.clone(),
            limiter: self.limiter.clone(),
            #[cfg(feature = "event")]
            emit: self.emit.clone(),
        }
    }
}

impl<S: Store> Passkeys<S> {
    /// `app_url` is the origin browsers see (`https://acme.test`, or
    /// `http://localhost:3000` in development); its host becomes the relying
    /// party id that passkeys are bound to. Changing the host later orphans
    /// every passkey. `app_name` is shown in the browser's passkey dialog.
    pub fn new(store: S, app_url: &str, app_name: &str) -> Result<Self, Error> {
        let origin = Url::parse(app_url).map_err(Error::other)?;
        let rp_id = origin.host_str().ok_or_else(|| Error::other("the app URL has no host"))?.to_owned();
        let webauthn = WebauthnBuilder::new(&rp_id, &origin).map_err(Error::other)?.rp_name(app_name).build().map_err(Error::other)?;
        Ok(Self::with_webauthn(store, webauthn))
    }

    /// For settings `new` does not take: extra origins, subdomains, timeouts.
    pub fn with_webauthn(store: S, webauthn: Webauthn) -> Self {
        Self {
            store: Arc::new(store),
            webauthn: Arc::new(webauthn),
            limiter: None,
            #[cfg(feature = "event")]
            emit: None,
        }
    }

    /// Limit failed answers: passwordless sign-ins per IP, second factors per
    /// user and IP. A success clears the count. Keep the limiter for the life
    /// of the app: a memory limiter made per request forgets every attempt.
    #[cfg(feature = "throttle")]
    pub fn throttle<A: throttle::Algorithm, St: throttle::Store>(mut self, limiter: Arc<throttle::Limiter<A, St>>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    /// Run `finish` under the limiter: refused while `key` is over the limit,
    /// counted when the answer fails, cleared when it passes.
    async fn limited<T>(&self, key: String, finish: impl Future<Output = Result<T, Error>>) -> Result<T, Error> {
        let Some(l) = &self.limiter else { return finish.await };
        if let Some(retry) = l.check(&key).await.map_err(Error::Other)? {
            return Err(Error::Throttled(retry));
        }
        let result = finish.await;
        match &result {
            Ok(_) => l.clear(&key).await.map_err(Error::Other)?,
            // A broken store is not the user's failure.
            Err(Error::Other(_)) => {}
            Err(_) => l.hit(&key).await.map_err(Error::Other)?,
        }
        result
    }

    /// Emit [`events`] on `events` with context `cx` (the app state) when a
    /// passkey is added, removed or used. A failing listener is logged; the
    /// change it reports has already happened.
    #[cfg(feature = "event")]
    pub fn events<C: Clone + Send + Sync + 'static>(mut self, events: Arc<event::Events<C>>, cx: C) -> Self {
        self.emit = Some(Arc::new(move |e| {
            let (events, cx) = (events.clone(), cx.clone());
            Box::pin(async move {
                match e {
                    Emitted::Added(e) => events.emit(&cx, e).await,
                    Emitted::Removed(e) => events.emit(&cx, e).await,
                    Emitted::Used(e) => events.emit(&cx, e).await,
                }
            })
        }));
        self
    }

    #[cfg_attr(not(feature = "event"), allow(clippy::unused_async))]
    async fn emit(&self, e: Emitted) {
        #[cfg(feature = "event")]
        if let Some(emit) = &self.emit
            && let Err(err) = emit(e).await
        {
            tracing::error!("passkeys event listener failed: {err}");
        }
        #[cfg(not(feature = "event"))]
        drop(e);
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    /// The user's passkeys, newest first.
    pub async fn passkeys(&self, user: i64) -> Result<Vec<Passkey>, Error> {
        self.store.passkeys(user).await
    }

    /// Start adding a passkey for a signed-in user. `user_name` (the email)
    /// and `display_name` label it in the browser and password manager.
    /// Asks for a discoverable credential, so the passkey can sign in without
    /// a username; the user's existing passkeys are excluded.
    pub async fn start_registration(&self, session: &Session, user: i64, user_name: &str, display_name: &str) -> Result<CreationChallengeResponse, Error> {
        let existing = self.store.passkeys(user).await?;
        // One handle per user, random: it is what a discoverable credential gives back at sign-in.
        let handle = existing.first().map_or_else(uuid::Uuid::new_v4, |p| p.user_handle);
        let exclude = existing.iter().map(|p| p.credential.cred_id().clone()).collect();
        let (mut options, state) = self.webauthn.start_passkey_registration(handle, user_name, display_name, Some(exclude)).map_err(Error::other)?;
        // webauthn-rs discourages resident keys; passwordless sign-in needs them. `Preferred`
        // lets security keys without storage still register (they then only work as a second factor).
        if let Some(selection) = options.public_key.authenticator_selection.as_mut() {
            selection.resident_key = Some(ResidentKeyRequirement::Preferred);
        }
        put(session, REGISTER, &(user, handle, state))?;
        Ok(options)
    }

    /// Check the browser's answer and store the passkey as `name`.
    pub async fn finish_registration(&self, session: &Session, user: i64, name: &str, answer: &RegisterPublicKeyCredential) -> Result<Passkey, Error> {
        let (owner, handle, state): (i64, uuid::Uuid, PasskeyRegistration) = take(session, REGISTER)?;
        if owner != user {
            return Err(Error::NoCeremony);
        }
        let credential = self.webauthn.finish_passkey_registration(answer, &state).map_err(Error::Rejected)?;
        let credential_id: Vec<u8> = credential.cred_id().to_vec();
        if self.store.by_credential_id(&credential_id).await?.is_some() {
            return Err(Error::AlreadyRegistered);
        }
        self.store
            .add(NewPasskey {
                public_id: Uuid::new(),
                user_id: user,
                name: name.trim().to_owned(),
                credential_id: credential_id.clone(),
                user_handle: handle,
                credential,
                created_at: now(),
            })
            .await?;
        let added = self.store.by_credential_id(&credential_id).await?.ok_or_else(|| Error::other("the new passkey was not found"))?;
        self.emit(Emitted::Added(events::PasskeyAdded { user_id: user, passkey: added.public_id, name: added.name.clone() })).await;
        Ok(added)
    }

    /// Second factor: after a correct password for `user` (the pending
    /// two-factor login), ask for one of their passkeys.
    pub async fn start_authentication(&self, session: &Session, user: i64) -> Result<RequestChallengeResponse, Error> {
        let credentials: Vec<Credential> = self.store.passkeys(user).await?.into_iter().map(|p| p.credential).collect();
        if credentials.is_empty() {
            return Err(Error::NoPasskeys);
        }
        let (options, state) = self.webauthn.start_passkey_authentication(&credentials).map_err(Error::other)?;
        put(session, SECOND_FACTOR, &(user, state))?;
        Ok(options)
    }

    /// Check the second factor. On success, finish the login (`TwoFactor::pass`, `auth.login`).
    /// Throttled per user and `ip`.
    pub async fn finish_authentication(&self, session: &Session, ip: Option<IpAddr>, user: i64, answer: &PublicKeyCredential) -> Result<Passkey, Error> {
        self.limited(format!("passkeys:second-factor:{user}|{}", ip_key(ip)), async {
            let (owner, state): (i64, PasskeyAuthentication) = take(session, SECOND_FACTOR)?;
            if owner != user {
                return Err(Error::NoCeremony);
            }
            let result = self.webauthn.finish_passkey_authentication(answer, &state).map_err(Error::Rejected)?;
            let passkey = self.store.by_credential_id(result.cred_id()).await?.filter(|p| p.user_id == user).ok_or(Error::UnknownCredential)?;
            self.used(passkey, &result, false).await
        })
        .await
    }

    /// Passwordless: options without a username; the browser lists the
    /// passkeys it has for this site. The options ask for conditional
    /// mediation (autofill); pass only `publicKey` to the browser for a modal prompt.
    pub fn start_login(&self, session: &Session) -> Result<RequestChallengeResponse, Error> {
        let (options, state) = self.webauthn.start_discoverable_authentication().map_err(Error::other)?;
        put(session, LOGIN, &state)?;
        Ok(options)
    }

    /// Check a passwordless sign-in. On success, log in `user_id` of the
    /// returned passkey: it verified the user (PIN or biometrics), so no
    /// second factor is asked. Throttled per `ip`.
    pub async fn finish_login(&self, session: &Session, ip: Option<IpAddr>, answer: &PublicKeyCredential) -> Result<Passkey, Error> {
        self.limited(format!("passkeys:login|{}", ip_key(ip)), async {
            let state: DiscoverableAuthentication = take(session, LOGIN)?;
            let (handle, credential_id) = self.webauthn.identify_discoverable_authentication(answer).map_err(Error::Rejected)?;
            let passkey = self.store.by_credential_id(credential_id).await?.filter(|p| p.user_handle == handle).ok_or(Error::UnknownCredential)?;
            let result = self.webauthn.finish_discoverable_authentication_v2(answer, state, &passkey.credential).map_err(Error::Rejected)?;
            self.used(passkey, &result, true).await
        })
        .await
    }

    /// Record a verified use: new counter and backup flags, `last_used_at`.
    async fn used(&self, mut passkey: Passkey, result: &AuthenticationResult, passwordless: bool) -> Result<Passkey, Error> {
        passkey.credential.update_credential(result);
        let at = now();
        self.store.used(passkey.id, &passkey.credential, at).await?;
        passkey.last_used_at = Some(at);
        self.emit(Emitted::Used(events::PasskeyUsed { user_id: passkey.user_id, passkey: passkey.public_id, passwordless })).await;
        Ok(passkey)
    }

    /// `false` if the user has no passkey with this id.
    pub async fn rename(&self, user: i64, passkey: Uuid<Passkey>, name: &str) -> Result<bool, Error> {
        self.store.rename(user, passkey, name.trim()).await
    }

    /// Gate it behind `auth::RecentlyConfirmed`. `false` if the user has no passkey with this id.
    pub async fn remove(&self, user: i64, passkey: Uuid<Passkey>) -> Result<bool, Error> {
        let removed = self.store.delete(user, passkey).await?;
        if removed {
            self.emit(Emitted::Removed(events::PasskeyRemoved { user_id: user, passkey })).await;
        }
        Ok(removed)
    }

    /// Remove all of the user's passkeys (before deleting the user). Emits nothing.
    pub async fn delete_all(&self, user: i64) -> Result<(), Error> {
        self.store.delete_all(user).await
    }
}

fn put<T: Serialize>(session: &Session, key: &str, state: &T) -> Result<(), Error> {
    session.insert(key, &Ceremony { state, expires_at: now() + CEREMONY_SECS }).map_err(Error::other)
}

/// The waiting ceremony, removed from the session: each state is used once.
fn take<T: DeserializeOwned>(session: &Session, key: &str) -> Result<T, Error> {
    let ceremony = session.get::<Ceremony<T>>(key).ok().flatten();
    session.remove(key);
    ceremony.filter(|c| c.expires_at > now()).map(|c| c.state).ok_or(Error::NoCeremony)
}

fn ip_key(ip: Option<IpAddr>) -> String {
    ip.map(|ip| ip.to_string()).unwrap_or_default()
}

fn now() -> i64 {
    // A clock set before 1970 reads as 0 rather than panicking.
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}
