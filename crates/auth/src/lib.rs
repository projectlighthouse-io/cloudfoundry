//! Know who the current user is, and guard routes.
//!
//! The app implements [`Provider`] (how to load a user by id) and adds
//! [`layer`] after the session layer. Then:
//!
//! - [`Authed<U>`]: handler input that exists only for a logged-in user.
//! - [`Auth<U>`]: optional user, plus `login` and `logout`.
//! - [`require`] / [`guest`]: layers for routes whose handlers take neither.
//! - [`Remember`] + `.remember(codec)`: "remember me" cookies.
//! - [`Tokens`] + `.tokens()`: `Authorization: Bearer` for APIs, with
//!   [`issue_token`] / [`revoke_token`]. Tokens can expire.
//! - [`Credentials`] + [`Auth::attempt`]: check a login and password, rehash
//!   old hashes, log in. With feature `throttle`, `.throttle(limiter)` limits
//!   failed attempts per login and IP.
//!
//! - [`Resets`] + [`reset_link_token`] / [`reset_password`]: "forgot password"
//!   links. Sending the mail is the app's.
//! - [`Auth::change_password`]: a logged-in user changes their password.
//!
//! Only the user id is stored in the session. The user is loaded at most once
//! per request, and only if something asks.

use std::{
    fmt,
    future::Future,
    net::IpAddr,
    ops::Deref,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use securecookie::Codec;

use route::{
    Body, ClientAddr, FromRequest, FromRequestParts, IntoResponse, Redirect, UrlFor,
    http::{Extensions, HeaderMap, HeaderValue, Request, Response, StatusCode, header, request::Parts},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use session::Session;
use tokio::sync::OnceCell;
use tower::{Layer, Service};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

const SESSION_KEY: &str = "_auth_id";

/// How the app loads its users. The only integration point.
pub trait Provider: Send + Sync + 'static {
    type User: Clone + Send + Sync + 'static;
    type Id: Serialize + DeserializeOwned + Send;

    fn find(&self, id: Self::Id) -> impl Future<Output = Result<Option<Self::User>, Error>> + Send;
    fn id(&self, user: &Self::User) -> Self::Id;
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// `Provider` with the id type erased to JSON, so lookups go by user type alone.
trait Erased<U>: Send + Sync {
    fn find(&self, id: Value) -> BoxFuture<'_, Result<Option<U>, Error>>;
    fn id(&self, user: &U) -> Value;
}

impl<P: Provider> Erased<P::User> for P {
    fn find(&self, id: Value) -> BoxFuture<'_, Result<Option<P::User>, Error>> {
        Box::pin(async move {
            // An id that no longer parses (the id type changed) is a logged-out user.
            match serde_json::from_value(id) {
                Ok(id) => Provider::find(self, id).await,
                Err(_) => Ok(None),
            }
        })
    }

    fn id(&self, user: &P::User) -> Value {
        // An id that does not serialize (a broken custom Serialize impl) logs the
        // user out on the next request instead of panicking.
        serde_json::to_value(Provider::id(self, user)).unwrap_or_else(|e| {
            tracing::error!("the user id does not serialize to JSON: {e}");
            Value::Null
        })
    }
}

/// Providers that keep a remember token per user, for "remember me".
pub trait Remember: Provider {
    /// The token stored for `user`, if any.
    fn remember_token(&self, user: &Self::User) -> Option<String>;
    /// Store a new token for the user with `id`, or clear it with `None`.
    fn set_remember_token(&self, id: Self::Id, token: Option<String>) -> impl Future<Output = Result<(), Error>> + Send;
}

/// Providers that keep API tokens, for `Authorization: Bearer`. Only a
/// SHA-256 hash of each token is stored ([`hash_token`]); the plain token is
/// shown to its owner once, by [`issue_token`].
pub trait Tokens: Provider {
    /// The owner of the token whose hash is `hash`, and when the token expires
    /// (unix seconds; `None` for never).
    fn by_token_hash(&self, hash: &str) -> impl Future<Output = Result<Option<TokenOwner<Self::User>>, Error>> + Send;
    /// Keep a new token for the user with `id`.
    fn store_token(&self, id: Self::Id, token: NewToken) -> impl Future<Output = Result<(), Error>> + Send;
    /// Forget the token whose hash is `hash`.
    fn delete_token(&self, hash: &str) -> impl Future<Output = Result<(), Error>> + Send;
}

/// A token's owner and when the token expires (unix seconds; `None` for never).
pub type TokenOwner<U> = (U, Option<i64>);

/// A token to store: its name (e.g. the device), hash and expiry.
#[derive(Clone, Debug)]
pub struct NewToken {
    pub name: String,
    pub hash: String,
    /// Unix seconds; `None` for a token that does not expire.
    pub expires_at: Option<i64>,
}

/// Issue an API token for `user`, valid for `ttl` (`None`: until revoked).
/// Returns the plain token: show it once; only its hash is stored.
pub async fn issue_token<P: Tokens>(users: &P, user: &P::User, name: &str, ttl: Option<Duration>) -> Result<String, Error> {
    let token = random_token();
    let expires_at = ttl.map(|ttl| unix_now() + ttl.as_secs() as i64);
    users.store_token(users.id(user), NewToken { name: name.to_owned(), hash: hash_token(&token), expires_at }).await?;
    Ok(token)
}

/// Revoke a plain token, e.g. the request's own ([`Auth::bearer_token`]) on API logout.
pub async fn revoke_token<P: Tokens>(users: &P, token: &str) -> Result<(), Error> {
    users.delete_token(&hash_token(token)).await
}

/// SHA-256 of `token`, lowercase hex: what [`Tokens`] stores and looks up.
pub fn hash_token(token: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn unix_now() -> i64 {
    // A clock set before 1970 reads as 0 rather than panicking.
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Providers that can check a password, for [`Auth::attempt`].
pub trait Credentials: Provider {
    /// The user who logs in as `login` (usually the email address).
    fn by_login(&self, login: &str) -> impl Future<Output = Result<Option<Self::User>, Error>> + Send;
    /// The user's stored `password::hash`; `None` for an account without a
    /// password (made through social login). Such accounts cannot log in with
    /// a password until one is set, e.g. through a password reset.
    fn password_hash(&self, user: &Self::User) -> Option<String>;
    /// Store a new hash for the user with `id`.
    fn set_password_hash(&self, id: Self::Id, hash: String) -> impl Future<Output = Result<(), Error>> + Send;
}

/// Why [`Auth::attempt`] did not log the user in.
#[derive(Debug)]
pub enum AttemptError {
    /// Unknown login or wrong password. Deliberately not told apart.
    Failed,
    /// Too many failed attempts for this login and IP; try again after this long.
    Throttled(Duration),
    /// The provider or throttle store failed.
    Other(Error),
}

impl fmt::Display for AttemptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed => f.write_str("these credentials do not match our records"),
            Self::Throttled(d) => write!(f, "too many login attempts; try again in {} seconds", d.as_secs().max(1)),
            Self::Other(e) => write!(f, "login failed: {e}"),
        }
    }
}

impl std::error::Error for AttemptError {}

/// Providers that keep password reset tokens: at most one per user, as a
/// SHA-256 hash with its creation time.
pub trait Resets: Credentials {
    /// Store the token hash for the user with `id`, replacing any older one.
    fn store_reset(&self, id: Self::Id, hash: String, created_at: i64) -> impl Future<Output = Result<(), Error>> + Send;
    /// The stored hash and creation time (unix seconds) for the user with `id`.
    fn find_reset(&self, id: Self::Id) -> impl Future<Output = Result<Option<(String, i64)>, Error>> + Send;
    fn delete_reset(&self, id: Self::Id) -> impl Future<Output = Result<(), Error>> + Send;
}

/// How long a reset link works.
pub const RESET_EXPIRES: Duration = Duration::from_secs(60 * 60);
/// How often a user can be sent a new link.
pub const RESET_THROTTLE: Duration = Duration::from_secs(60);

/// Why a reset step failed.
#[derive(Debug)]
pub enum ResetError {
    /// No user has that login. Show the same message as success, so the form
    /// does not reveal who has an account.
    UnknownUser,
    /// A link was made less than [`RESET_THROTTLE`] ago.
    Throttled,
    /// The token is wrong, used or older than [`RESET_EXPIRES`].
    InvalidToken,
    Other(Error),
}

impl fmt::Display for ResetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownUser => f.write_str("no user with that login"),
            Self::Throttled => f.write_str("a reset link was sent moments ago"),
            Self::InvalidToken => f.write_str("this password reset token is invalid"),
            Self::Other(e) => write!(f, "password reset failed: {e}"),
        }
    }
}

impl std::error::Error for ResetError {}

/// Make a reset token for `login`. Returns the user (to address the mail)
/// and the plain token to put in the link; only its hash is stored.
pub async fn reset_link_token<P: Resets>(users: &P, login: &str) -> Result<(P::User, String), ResetError> {
    let user = users.by_login(login).await.map_err(ResetError::Other)?.ok_or(ResetError::UnknownUser)?;
    let now = unix_now();
    if let Some((_, created)) = users.find_reset(users.id(&user)).await.map_err(ResetError::Other)? {
        if now - created < RESET_THROTTLE.as_secs() as i64 {
            return Err(ResetError::Throttled);
        }
    }
    let token = random_token();
    users.store_reset(users.id(&user), hash_token(&token), now).await.map_err(ResetError::Other)?;
    Ok((user, token))
}

/// Set a new password if `token` is the user's current, unexpired reset
/// token. The token is used up. Returns the user, e.g. to log them in.
pub async fn reset_password<P: Resets>(users: &P, login: &str, token: &str, new_password: &str) -> Result<P::User, ResetError> {
    let user = users.by_login(login).await.map_err(ResetError::Other)?.ok_or(ResetError::InvalidToken)?;
    let stored = users.find_reset(users.id(&user)).await.map_err(ResetError::Other)?;
    let fresh = |created: i64| unix_now() - created < RESET_EXPIRES.as_secs() as i64;
    if !stored.is_some_and(|(hash, created)| ct_eq(&hash, &hash_token(token)) && fresh(created)) {
        return Err(ResetError::InvalidToken);
    }
    let hash = hash_password(new_password).await.map_err(ResetError::Other)?;
    users.set_password_hash(users.id(&user), hash).await.map_err(ResetError::Other)?;
    users.delete_reset(users.id(&user)).await.map_err(ResetError::Other)?;
    Ok(user)
}

/// Why [`Auth::change_password`] did not change the password.
#[derive(Debug)]
pub enum ChangeError {
    /// Nobody is logged in.
    NotLoggedIn,
    /// The account has no password yet (social login); set one with a reset.
    NoPassword,
    /// The current password is wrong.
    WrongPassword,
    Other(Error),
}

impl fmt::Display for ChangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLoggedIn => f.write_str("not logged in"),
            Self::NoPassword => f.write_str("the account has no password"),
            Self::WrongPassword => f.write_str("the current password is incorrect"),
            Self::Other(e) => write!(f, "changing the password failed: {e}"),
        }
    }
}

impl std::error::Error for ChangeError {}

/// Failed-login limiter with its algorithm and store erased.
trait LoginLimiter: Send + Sync {
    /// `Some(retry_after)` when `key` may not try now.
    fn check<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Duration>, Error>>;
    fn hit<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), Error>>;
    fn clear<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), Error>>;
}

#[cfg(feature = "throttle")]
impl<A: throttle::Algorithm, St: throttle::Store> LoginLimiter for throttle::Limiter<A, St> {
    fn check<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Duration>, Error>> {
        Box::pin(async move { Ok(Some(throttle::Limiter::check(self, key).await?).filter(|d| !d.allowed).map(|d| d.retry_after)) })
    }

    fn hit<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move { throttle::Limiter::hit(self, key).await.map(drop) })
    }

    fn clear<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(throttle::Limiter::clear(self, key))
    }
}

trait ErasedRemember<U>: Send + Sync {
    fn token_of(&self, user: &U) -> Option<String>;
    fn set(&self, id: Value, token: Option<String>) -> BoxFuture<'_, Result<(), Error>>;
}

impl<P: Remember> ErasedRemember<P::User> for P {
    fn token_of(&self, user: &P::User) -> Option<String> {
        self.remember_token(user)
    }

    fn set(&self, id: Value, token: Option<String>) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move { self.set_remember_token(serde_json::from_value(id)?, token).await })
    }
}

trait ErasedTokens<U>: Send + Sync {
    fn by_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<Option<U>, Error>>;
}

impl<P: Tokens> ErasedTokens<P::User> for P {
    fn by_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<Option<P::User>, Error>> {
        Box::pin(async move {
            let found = self.by_token_hash(&hash_token(token)).await?;
            Ok(found.filter(|(_, exp)| exp.is_none_or(|exp| exp > unix_now())).map(|(user, _)| user))
        })
    }
}

/// Browsers cap cookie lifetimes at 400 days.
const MAX_COOKIE_AGE: Duration = Duration::from_secs(400 * 86400);

struct RememberCfg<U> {
    store: Arc<dyn ErasedRemember<U>>,
    codec: Arc<Codec>,
}

/// Cookie work the layer does after the handler.
#[derive(Default)]
enum Pending {
    #[default]
    None,
    Remember { id: Value, token: String },
    Forget { id: Option<Value> },
}

/// Per-request auth context, in request extensions.
struct AuthCx<U> {
    provider: Arc<dyn Erased<U>>,
    remember: Option<Arc<RememberCfg<U>>>,
    tokens: Option<Arc<dyn ErasedTokens<U>>>,
    throttle: Option<Arc<dyn LoginLimiter>>,
    ip: Option<IpAddr>,
    cfg: Arc<Config>,
    user: Arc<OnceCell<Option<U>>>,
    bearer: Option<Arc<str>>,
    recaller: Option<Arc<str>>,
    pending: Arc<Mutex<Pending>>,
}

impl<U> Clone for AuthCx<U> {
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            remember: self.remember.clone(),
            tokens: self.tokens.clone(),
            throttle: self.throttle.clone(),
            ip: self.ip,
            cfg: self.cfg.clone(),
            user: self.user.clone(),
            bearer: self.bearer.clone(),
            recaller: self.recaller.clone(),
            pending: self.pending.clone(),
        }
    }
}

/// Layer settings. Deserializable with feature `configured`; missing keys keep
/// their defaults, unknown keys are an error:
///
/// ```toml
/// [auth]
/// login_route = "login"
/// secure = true
/// remember_cookie = "remember"
/// remember_for = "30d"
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "configured",
    derive(configured::serde::Deserialize, configured::serde::Serialize),
    serde(crate = "configured::serde", default, deny_unknown_fields)
)]
pub struct Config {
    /// Route name anonymous page visits are sent to.
    pub login_route: String,
    /// `Secure` on the remember cookie. Browsers treat `localhost` as secure.
    pub secure: bool,
    /// Name of the remember cookie.
    pub remember_cookie: String,
    /// How long a remember cookie lasts. At most 400 days (the browser cap).
    #[cfg_attr(feature = "configured", serde(with = "configured::duration"))]
    pub remember_for: Duration,
}

/// `login`, `Secure`, cookie `remember`, 400 days.
impl Default for Config {
    fn default() -> Self {
        Self { login_route: "login".into(), secure: true, remember_cookie: "remember".into(), remember_for: MAX_COOKIE_AGE }
    }
}

/// A [`Config`] that would break login redirects or the remember cookie.
#[derive(Debug)]
pub struct ConfigError(String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "auth config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Run by [`AuthLayer::config`].
    pub fn check(&self) -> Result<(), ConfigError> {
        let err = |m: String| Err(ConfigError(m));
        if self.login_route.is_empty() {
            return err("login_route is empty".into());
        }
        let token = |c: u8| c.is_ascii_graphic() && !b"()<>@,;:\\\"/[]?={}".contains(&c);
        if self.remember_cookie.is_empty() || !self.remember_cookie.bytes().all(token) {
            return err(format!("remember_cookie `{}` is not a valid cookie token", self.remember_cookie));
        }
        if self.remember_for.as_secs() == 0 || self.remember_for > MAX_COOKIE_AGE {
            return err("remember_for must be between 1 second and 400 days".into());
        }
        Ok(())
    }
}

impl<U> AuthCx<U> {
    /// Cookie work for after the handler, even if a panic elsewhere poisoned the lock.
    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl<U: Clone + Send + Sync + 'static> AuthCx<U> {

    /// Bearer token first (APIs), then the session, then the remember cookie.
    async fn user(&self, session: Option<&Session>) -> Result<Option<U>, Error> {
        self.user
            .get_or_try_init(|| async {
                if let (Some(tokens), Some(bearer)) = (&self.tokens, &self.bearer) {
                    return tokens.by_token(bearer).await;
                }
                let Some(session) = session else { return Ok(None) };
                match session.get::<Value>(SESSION_KEY)? {
                    Some(id) => self.provider.find(id).await,
                    None => self.recall(session).await,
                }
            })
            .await
            .cloned()
    }

    /// Log in from a valid remember cookie whose token still matches the stored one.
    async fn recall(&self, session: &Session) -> Result<Option<U>, Error> {
        let (Some(r), Some(cookie)) = (&self.remember, &self.recaller) else { return Ok(None) };
        let mut found = None;
        if let Ok((id, token)) = r.codec.verify::<(Value, String)>(&self.cfg.remember_cookie, cookie, self.cfg.remember_for) {
            if let Some(user) = self.provider.find(id.clone()).await? {
                if r.store.token_of(&user).is_some_and(|t| ct_eq(&t, &token)) {
                    found = Some((id, user));
                }
            }
        }
        if let Some((id, user)) = found {
            session.regenerate();
            session.insert(SESSION_KEY, &id)?;
            Ok(Some(user))
        } else {
            // Stale or forged: drop the cookie.
            *self.pending() = Pending::Forget { id: None };
            Ok(None)
        }
    }

    async fn finish<B>(&self, res: &mut Response<B>) {
        let Some(r) = &self.remember else { return };
        let pending = std::mem::take(&mut *self.pending());
        match pending {
            Pending::None => {}
            Pending::Remember { id, token } => {
                if let Err(e) = r.store.set(id.clone(), Some(token.clone())).await {
                    return tracing::error!("storing the remember token failed: {e}");
                }
                match r.codec.sign(&self.cfg.remember_cookie, &(id, token)) {
                    Ok(v) => set_cookie(res, &self.cfg.remember_cookie, &v, self.cfg.remember_for.as_secs(), self.cfg.secure),
                    Err(e) => tracing::error!("signing the remember cookie failed: {e}"),
                }
            }
            Pending::Forget { id } => {
                if let Some(id) = id {
                    if let Err(e) = r.store.set(id, None).await {
                        tracing::error!("clearing the remember token failed: {e}");
                    }
                }
                set_cookie(res, &self.cfg.remember_cookie, "", 0, self.cfg.secure);
            }
        }
    }
}

fn set_cookie<B>(res: &mut Response<B>, name: &str, value: &str, max_age: u64, secure: bool) {
    let secure = if secure { "; Secure" } else { "" };
    let c = format!("{name}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}");
    if let Ok(v) = HeaderValue::try_from(c) {
        res.headers_mut().append(header::SET_COOKIE, v);
    }
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ").map(str::trim)
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random_token() -> String {
    let mut b = [0u8; 32];
    // No secure fallback exists: a guessable token must not be issued.
    #[allow(clippy::expect_used, reason = "the OS random source failed; there is no safe fallback")]
    getrandom::fill(&mut b).expect("OS random source");
    URL_SAFE_NO_PAD.encode(b)
}

#[allow(clippy::result_large_err, reason = "the error is the response itself, returned once per request")]
fn context<U: Send + Sync + 'static>(ext: &Extensions) -> Result<(AuthCx<U>, Option<Session>), Response<Body>> {
    if let Some(cx) = ext.get::<AuthCx<U>>() {
        Ok((cx.clone(), ext.get::<Session>().cloned()))
    } else {
        tracing::error!("no auth context; add `auth::layer` (after `session::layer`) to the router");
        Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }
}

async fn current<U: Clone + Send + Sync + 'static>(ext: &Extensions) -> Result<(Option<U>, AuthCx<U>, Option<Session>), Response<Body>> {
    let (cx, session) = context::<U>(ext)?;
    match cx.user(session.as_ref()).await {
        Ok(user) => Ok((user, cx, session)),
        Err(e) => {
            tracing::error!("loading the current user failed: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

/// Redirect page visits to the login route; `401` for JSON clients.
fn unauthenticated(ext: &Extensions, headers: &HeaderMap, login: &str) -> Response<Body> {
    let inertia = headers.contains_key("x-inertia");
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"));
    let url = ext.get::<UrlFor>().and_then(|u| u.url_by_name(login, &[]).ok());
    match url {
        Some(url) if inertia || !wants_json => Redirect::to(url).into_response(),
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Make `Auth<U>`, `Authed<U>`, `require` and `guest` work. Add after the
/// session layer.
pub fn layer<P: Provider>(provider: P) -> AuthLayer<P> {
    AuthLayer { provider: Arc::new(provider), remember: None, tokens: None, throttle: None, cfg: Arc::new(Config::default()) }
}

pub struct AuthLayer<P: Provider> {
    provider: Arc<P>,
    remember: Option<Arc<RememberCfg<P::User>>>,
    tokens: Option<Arc<dyn ErasedTokens<P::User>>>,
    throttle: Option<Arc<dyn LoginLimiter>>,
    cfg: Arc<Config>,
}

impl<P: Provider> AuthLayer<P> {
    /// Replace all settings after [`Config::check`]. Setters called after
    /// this change single fields and are not checked.
    pub fn config(mut self, cfg: Config) -> Result<Self, ConfigError> {
        cfg.check()?;
        self.cfg = Arc::new(cfg);
        Ok(self)
    }

    /// Route name anonymous page visits are sent to. Default `login`.
    pub fn login_route(mut self, name: &str) -> Self {
        Arc::make_mut(&mut self.cfg).login_route = name.to_owned();
        self
    }

    /// `Secure` on the remember cookie. Default `true` (browsers accept it on localhost).
    pub fn secure(mut self, secure: bool) -> Self {
        Arc::make_mut(&mut self.cfg).secure = secure;
        self
    }
}

#[cfg(feature = "throttle")]
impl<P: Credentials> AuthLayer<P> {
    /// Limit failed [`Auth::attempt`]s per login and client IP. A success
    /// clears the count. Laravel's default is 5 per minute:
    /// `.throttle(Limiter::memory(FixedWindow::new(5, Duration::from_secs(60))))`.
    pub fn throttle<A: throttle::Algorithm, St: throttle::Store>(mut self, limiter: throttle::Limiter<A, St>) -> Self {
        self.throttle = Some(Arc::new(limiter));
        self
    }
}

impl<P: Remember> AuthLayer<P> {
    /// Enable "remember me": `Auth::login_remember` sets a signed cookie
    /// (`Config::remember_for`, default 400 days) that logs the user back in after the session ends. Logout
    /// clears the stored token, so old cookies stop working.
    pub fn remember(mut self, codec: Codec) -> Self {
        let store: Arc<dyn ErasedRemember<P::User>> = self.provider.clone();
        self.remember = Some(Arc::new(RememberCfg { store, codec: Arc::new(codec) }));
        self
    }
}

impl<P: Tokens> AuthLayer<P> {
    /// Accept `Authorization: Bearer <token>`. A bearer request is
    /// authenticated by the token alone; the session is not consulted, and an
    /// expired token is no token. Exempt such API routes from CSRF, which
    /// guards cookie-based requests.
    pub fn tokens(mut self) -> Self {
        let tokens: Arc<dyn ErasedTokens<P::User>> = self.provider.clone();
        self.tokens = Some(tokens);
        self
    }
}

impl<P: Provider> Clone for AuthLayer<P> {
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            remember: self.remember.clone(),
            tokens: self.tokens.clone(),
            throttle: self.throttle.clone(),
            cfg: self.cfg.clone(),
        }
    }
}

impl<S, P: Provider> Layer<S> for AuthLayer<P> {
    type Service = AuthService<S, P>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService { inner, layer: self.clone() }
    }
}

pub struct AuthService<S, P: Provider> {
    inner: S,
    layer: AuthLayer<P>,
}

impl<S: Clone, P: Provider> Clone for AuthService<S, P> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), layer: self.layer.clone() }
    }
}

impl<S, P, ReqB, ResB> Service<Request<ReqB>> for AuthService<S, P>
where
    S: Service<Request<ReqB>, Response = Response<ResB>> + Clone + Send + 'static,
    S::Future: Send,
    P: Provider,
    ReqB: Send + 'static,
    ResB: Send + 'static,
{
    type Response = Response<ResB>;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Response<ResB>, S::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqB>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let l = &self.layer;
        let provider: Arc<dyn Erased<P::User>> = l.provider.clone();
        let cx = AuthCx::<P::User> {
            provider,
            remember: l.remember.clone(),
            tokens: l.tokens.clone(),
            throttle: l.throttle.clone(),
            ip: req.extensions().get::<ClientAddr>().map(|a| a.0.ip()),
            cfg: l.cfg.clone(),
            user: Arc::new(OnceCell::new()),
            bearer: l.tokens.as_ref().and_then(|_| bearer(req.headers())).map(Into::into),
            recaller: l.remember.as_ref().and_then(|_| cookie(req.headers(), &l.cfg.remember_cookie)).map(Into::into),
            pending: Arc::default(),
        };
        req.extensions_mut().insert(cx.clone());
        Box::pin(async move {
            let mut res = inner.call(req).await?;
            cx.finish(&mut res).await;
            Ok(res)
        })
    }
}

/// The logged-in user. A handler taking one never runs for an anonymous
/// request, even on a route without `require`.
pub struct Authed<U>(pub U);

impl<U> Deref for Authed<U> {
    type Target = U;
    fn deref(&self) -> &U {
        &self.0
    }
}

impl<S: Send + Sync, U: Clone + Send + Sync + 'static> FromRequestParts<S> for Authed<U> {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        match current::<U>(&parts.extensions).await? {
            (Some(user), ..) => Ok(Authed(user)),
            (None, cx, _) => Err(unauthenticated(&parts.extensions, &parts.headers, &cx.cfg.login_route)),
        }
    }
}

impl<S: Send + Sync, U: Clone + Send + Sync + 'static> FromRequest<S> for Authed<U> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}

/// The user if there is one, plus login and logout. Needs the session layer.
pub struct Auth<U> {
    user: Option<U>,
    cx: AuthCx<U>,
    session: Session,
}

impl<U> Auth<U> {
    pub fn user(&self) -> Option<&U> {
        self.user.as_ref()
    }

    /// The `Authorization: Bearer` token of this request (with `.tokens()`).
    pub fn bearer_token(&self) -> Option<&str> {
        self.cx.bearer.as_deref()
    }

    /// Log `user` in. Regenerates the session id.
    pub fn login(&self, user: &U) {
        self.session.regenerate();
        if let Err(e) = self.session.insert(SESSION_KEY, &self.cx.provider.id(user)) {
            tracing::error!("storing the logged-in user failed: {e}");
        }
    }

    /// `login`, plus a remember cookie (needs `.remember(codec)` on the layer).
    pub fn login_remember(&self, user: &U) {
        self.login(user);
        if self.cx.remember.is_none() {
            return tracing::warn!("login_remember without `.remember(codec)` on the auth layer; no cookie set");
        }
        *self.cx.pending() = Pending::Remember { id: self.cx.provider.id(user), token: random_token() };
    }

    /// Log out. Invalidates the whole session and, with remember enabled,
    /// clears the stored token and the cookie.
    pub fn logout(&self) {
        self.session.invalidate();
        if self.cx.remember.is_some() {
            let id = self.user.as_ref().map(|u| self.cx.provider.id(u));
            *self.cx.pending() = Pending::Forget { id };
        }
    }
}

impl<U: Clone + Send + Sync + 'static> Auth<U> {
    /// Log in the user whose login and password match: the throttle check
    /// (with `.throttle`), the lookup, `password::verify` off the async
    /// runtime, a rehash when the stored hash uses old settings, then
    /// [`login`](Self::login) or [`login_remember`](Self::login_remember).
    /// Write the handler around it; add checks of your own before or after.
    pub async fn attempt<P>(&self, users: &P, login: &str, password: &str, remember: bool) -> Result<U, AttemptError>
    where
        P: Credentials<User = U>,
    {
        let key = format!("login:{}|{}", login.trim().to_lowercase(), self.cx.ip.map(|ip| ip.to_string()).unwrap_or_default());
        let throttle = self.cx.throttle.as_deref();
        if let Some(t) = throttle {
            if let Some(retry) = t.check(&key).await.map_err(AttemptError::Other)? {
                return Err(AttemptError::Throttled(retry));
            }
        }

        let user = users.by_login(login).await.map_err(AttemptError::Other)?;
        let hash = user.as_ref().and_then(|u| users.password_hash(u));
        let pw = password.to_owned();
        let matched = blocking(move || {
            if let Some(hash) = hash {
                password::verify(&pw, &hash).is_ok().then_some(hash)
            } else {
                password::verify_none(&pw);
                None
            }
        })
        .await?;

        let (Some(user), Some(hash)) = (user, matched) else {
            if let Some(t) = throttle {
                t.hit(&key).await.map_err(AttemptError::Other)?;
            }
            return Err(AttemptError::Failed);
        };
        if let Some(t) = throttle {
            t.clear(&key).await.map_err(AttemptError::Other)?;
        }

        if password::needs_rehash(&hash) {
            let pw = password.to_owned();
            match blocking(move || password::hash(&pw)).await? {
                Ok(new) => {
                    if let Err(e) = users.set_password_hash(users.id(&user), new).await {
                        tracing::warn!("storing the rehashed password failed: {e}");
                    }
                }
                Err(e) => tracing::warn!("rehashing the password failed: {e}"),
            }
        }

        if remember {
            self.login_remember(&user);
        } else {
            self.login(&user);
        }
        Ok(user)
    }
}

impl<U: Clone + Send + Sync + 'static> Auth<U> {
    /// Change the logged-in user's password once `current` checks out. The
    /// session gets a new id and, with "remember me", the remember token is
    /// cleared, so a stolen session or remember cookie stops working. The
    /// user stays logged in on this device.
    pub async fn change_password<P>(&self, users: &P, current: &str, new: &str) -> Result<(), ChangeError>
    where
        P: Credentials<User = U>,
    {
        let user = self.user.clone().ok_or(ChangeError::NotLoggedIn)?;
        let hash = users.password_hash(&user).ok_or(ChangeError::NoPassword)?;
        let pw = current.to_owned();
        let ok = tokio::task::spawn_blocking(move || password::verify(&pw, &hash).is_ok()).await.map_err(|e| ChangeError::Other(e.into()))?;
        if !ok {
            return Err(ChangeError::WrongPassword);
        }
        let new_hash = hash_password(new).await.map_err(ChangeError::Other)?;
        users.set_password_hash(users.id(&user), new_hash).await.map_err(ChangeError::Other)?;
        if self.cx.remember.is_some() {
            *self.cx.pending() = Pending::Forget { id: Some(self.cx.provider.id(&user)) };
        }
        self.login(&user);
        Ok(())
    }
}

/// `password::hash` off the async runtime, for registration and password
/// changes.
pub async fn hash_password(password: &str) -> Result<String, Error> {
    let pw = password.to_owned();
    Ok(tokio::task::spawn_blocking(move || password::hash(&pw)).await??)
}

/// Password hashing is CPU-heavy; keep it off the async worker threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, AttemptError> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AttemptError::Other(e.into()))
}

impl<S: Send + Sync, U: Clone + Send + Sync + 'static> FromRequestParts<S> for Auth<U> {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        let (user, cx, session) = current::<U>(&parts.extensions).await?;
        let Some(session) = session else {
            tracing::error!("`Auth` needs a session; add `session::layer` before `auth::layer`");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        };
        Ok(Auth { user, cx, session })
    }
}

impl<S: Send + Sync, U: Clone + Send + Sync + 'static> FromRequest<S> for Auth<U> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}

/// Only logged-in users pass. Others get the login redirect or a `401`.
pub fn require<U>() -> Guard<U> {
    Guard { kind: Kind::Require, _user: std::marker::PhantomData }
}

/// Only anonymous users pass. Logged-in users are redirected to `home`.
pub fn guest<U>() -> Guard<U> {
    Guard { kind: Kind::Guest("home"), _user: std::marker::PhantomData }
}

#[derive(Clone, Copy)]
enum Kind {
    Require,
    Guest(&'static str),
}

pub struct Guard<U> {
    kind: Kind,
    _user: std::marker::PhantomData<fn() -> U>,
}

impl<U> Guard<U> {
    /// For `guest`: where logged-in users go. Default `home`.
    pub fn redirect_to(self, route: &'static str) -> Self {
        Guard { kind: Kind::Guest(route), _user: std::marker::PhantomData }
    }
}

impl<U> Clone for Guard<U> {
    fn clone(&self) -> Self {
        Guard { kind: self.kind, _user: std::marker::PhantomData }
    }
}

impl<S, U> Layer<S> for Guard<U> {
    type Service = GuardService<S, U>;

    fn layer(&self, inner: S) -> Self::Service {
        GuardService { inner, guard: self.clone() }
    }
}

pub struct GuardService<S, U> {
    inner: S,
    guard: Guard<U>,
}

impl<S: Clone, U> Clone for GuardService<S, U> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), guard: self.guard.clone() }
    }
}

impl<S, U, ReqB> Service<Request<ReqB>> for GuardService<S, U>
where
    S: Service<Request<ReqB>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send,
    U: Clone + Send + Sync + 'static,
    ReqB: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqB>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let kind = self.guard.kind;
        Box::pin(async move {
            let (user, cx, _) = match current::<U>(req.extensions()).await {
                Ok(c) => c,
                Err(res) => return Ok(res),
            };
            match (kind, user.is_some()) {
                (Kind::Require, false) => Ok(unauthenticated(req.extensions(), req.headers(), &cx.cfg.login_route)),
                (Kind::Guest(home), true) => Ok(match req.extensions().get::<UrlFor>().and_then(|u| u.url(home, &[]).ok()) {
                    Some(url) => Redirect::to(url).into_response(),
                    None => StatusCode::FORBIDDEN.into_response(),
                }),
                _ => inner.call(req).await,
            }
        })
    }
}
