//! Two-factor authentication, Laravel Fortify-style: TOTP (SHA-1, 6 digits,
//! 30 s), single-use backup codes and "trust this browser" cookies.
//!
//! [`TwoFactor`] holds the rules over a [`Store`]. It does not log anyone in:
//! the published login handler checks the password with `auth.validate`,
//! then either logs in or calls [`TwoFactor::begin`]; the challenge handler
//! calls [`TwoFactor::challenge`] and, when it passes, `auth.login`.
//!
//! ```ignore
//! let tf = TwoFactor::new(store, crypt, cookie_codec, "Acme");
//! let setup = tf.setup(user.id, &user.email).await?;   // QR + otpauth:// URI, unconfirmed
//! let backup_codes = tf.confirm(user.id, "123456").await?; // show once
//! ```
//!
//! - Codes are accepted for the current time step and one either side, and
//!   each step only once: a code seen before is rejected.
//! - Secrets are stored sealed with `crypt`, bound to the user id.
//! - Backup codes are stored as SHA-256 hashes and work once.
//! - A trusted browser carries a signed cookie whose token is stored hashed.

use std::{
    fmt,
    future::Future,
    net::IpAddr,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use route::{
    Body, FromRequest, FromRequestParts,
    http::{HeaderMap, Request, Response, header, request::Parts},
};
use serde::{Deserialize, Serialize};
use session::Session;
use sha2::{Digest, Sha256};
use totp_rs::{Algorithm, Secret, TOTP};

pub use db::Uuid;
pub use store::{MemoryStore, NewDevice, Store, StoredSecret, TrustedDevice};
#[cfg(feature = "mysql")]
pub use sql::MysqlStore;
#[cfg(feature = "postgres")]
pub use sql::PostgresStore;
#[cfg(feature = "sqlite")]
pub use sql::SqliteStore;

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]
mod sql;
mod store;

/// The trusted-browser cookie.
pub const TRUST_COOKIE: &str = "two_factor_trusted";
/// The pending login, in the session.
const PENDING_KEY: &str = "_factor2_pending";
const DIGITS: usize = 6;
const STEP: u64 = 30;
/// Steps accepted either side of now.
const SKEW: u8 = 1;
const SECRET_BYTES: usize = 20;

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Settings with defaults. Deserializable with feature `configured`; missing
/// keys keep their defaults, unknown keys are an error:
///
/// ```toml
/// [two_factor]
/// trust_for = "30d"
/// pending_for = "5m"
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "configured",
    derive(configured::serde::Deserialize, configured::serde::Serialize),
    serde(crate = "configured::serde", default, deny_unknown_fields)
)]
pub struct Config {
    /// How long "trust this browser" lasts. At most 400 days (the cookie cap).
    #[cfg_attr(feature = "configured", serde(with = "configured::duration"))]
    pub trust_for: Duration,
    /// How long a correct password waits for its code.
    #[cfg_attr(feature = "configured", serde(with = "configured::duration"))]
    pub pending_for: Duration,
    /// How many backup codes a user gets.
    pub backup_codes: usize,
    /// `Secure` on the trusted-browser cookie. Browsers treat `localhost` as secure.
    pub secure: bool,
}

/// 30 days, 5 minutes, 10 codes, `Secure`.
impl Default for Config {
    fn default() -> Self {
        Self { trust_for: Duration::from_secs(30 * 86400), pending_for: Duration::from_secs(5 * 60), backup_codes: 10, secure: true }
    }
}

/// A [`Config`] no flow can use.
#[derive(Debug)]
pub struct ConfigError(&'static str);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "factor2 config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Run by [`TwoFactor::config`].
    pub fn check(&self) -> Result<(), ConfigError> {
        if self.trust_for.is_zero() || self.trust_for > Duration::from_secs(400 * 86400) {
            return Err(ConfigError("trust_for must be between 1 second and 400 days"));
        }
        if self.pending_for.is_zero() {
            return Err(ConfigError("pending_for must be above zero"));
        }
        if self.backup_codes == 0 {
            return Err(ConfigError("backup_codes must be at least 1"));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum Error {
    /// The user has no confirmed two-factor secret.
    NotEnabled,
    /// Two-factor is already on; disable it first to start over.
    AlreadyEnabled,
    /// The code is wrong, expired or was used before.
    InvalidCode,
    /// The store, encryption or QR rendering failed.
    Other(BoxError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnabled => f.write_str("two-factor authentication is not enabled"),
            Self::AlreadyEnabled => f.write_str("two-factor authentication is already enabled"),
            Self::InvalidCode => f.write_str("the code is invalid"),
            Self::Other(e) => write!(f, "two-factor: {e}"),
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

/// Why [`TwoFactor::challenge`] did not pass.
#[derive(Debug)]
pub enum ChallengeError {
    /// No password was checked, or it was more than `Config::pending_for` ago: log in again.
    NoPending,
    /// Too many wrong codes for this user and IP; try again after this long.
    Throttled(Duration),
    /// Wrong or reused code. The pending login stays, so the user can retry.
    InvalidCode { user_id: i64 },
    Other(Error),
}

impl fmt::Display for ChallengeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPending => f.write_str("no login is waiting for a code"),
            Self::Throttled(d) => write!(f, "too many attempts; try again in {} seconds", d.as_secs().max(1)),
            Self::InvalidCode { .. } => f.write_str("the code is invalid"),
            Self::Other(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ChallengeError {}

/// What the user scans or types to add the account to an authenticator app.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Setup {
    /// Base32, for typing in by hand.
    pub secret: String,
    /// `otpauth://totp/...`
    pub uri: String,
    /// The URI as a QR code, an `<svg>` element.
    pub qr_svg: String,
}

/// Where a user stands, for the settings page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Status {
    /// A confirmed secret: logins ask for a code.
    pub enabled: bool,
    /// A secret waits for its first code.
    pub confirming: bool,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub backup_codes_left: usize,
}

/// A code typed at the challenge.
#[derive(Clone, Copy, Debug)]
pub enum Code<'a> {
    /// Six digits from the authenticator app.
    Totp(&'a str),
    /// One of the backup codes.
    Backup(&'a str),
}

/// A passed challenge: log `user_id` in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Passed {
    pub user_id: i64,
    /// The user ticked "remember me" at login.
    pub remember: bool,
    /// `Some(left)` when a backup code was used.
    pub backup_codes_left: Option<usize>,
}

/// A checked password waiting for its code, in the session.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Pending {
    user_id: i64,
    remember: bool,
    expires_at: i64,
}

/// Events for the app to emit with `event` (the published handlers do).
pub mod events {
    use std::net::IpAddr;

    #[derive(Clone, Debug)]
    pub struct TwoFactorEnabled {
        pub user_id: i64,
    }

    #[derive(Clone, Debug)]
    pub struct TwoFactorDisabled {
        pub user_id: i64,
    }

    #[derive(Clone, Debug)]
    pub struct TwoFactorChallengeFailed {
        pub user_id: i64,
        pub ip: Option<IpAddr>,
    }

    #[derive(Clone, Debug)]
    pub struct BackupCodeUsed {
        pub user_id: i64,
        pub remaining: usize,
    }
}

/// Failed-challenge limiter with its algorithm and store erased.
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
        Box::pin(async move { throttle::Limiter::hit(self, key).await.map(drop).map_err(Into::into) })
    }
    fn clear<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), BoxError>> {
        Box::pin(async move { throttle::Limiter::clear(self, key).await.map_err(Into::into) })
    }
}

/// The rules over a [`Store`]. Cheap to clone.
pub struct TwoFactor<S> {
    store: Arc<S>,
    crypt: Arc<crypt::Encrypter>,
    cookies: Arc<securecookie::Codec>,
    issuer: Arc<str>,
    cfg: Config,
    limiter: Option<Arc<dyn Limiter>>,
}

impl<S> Clone for TwoFactor<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            crypt: self.crypt.clone(),
            cookies: self.cookies.clone(),
            issuer: self.issuer.clone(),
            cfg: self.cfg.clone(),
            limiter: self.limiter.clone(),
        }
    }
}

impl<S: Store> TwoFactor<S> {
    /// `crypt` seals secrets; `cookies` signs the trusted-browser cookie;
    /// `issuer` is the name authenticator apps show (your app's name).
    pub fn new(store: S, crypt: crypt::Encrypter, cookies: securecookie::Codec, issuer: &str) -> Self {
        Self {
            store: Arc::new(store),
            crypt: Arc::new(crypt),
            cookies: Arc::new(cookies),
            issuer: issuer.into(),
            cfg: Config::default(),
            limiter: None,
        }
    }

    pub fn config(mut self, cfg: Config) -> Result<Self, ConfigError> {
        cfg.check()?;
        self.cfg = cfg;
        Ok(self)
    }

    /// Limit failed challenges per user and IP. Keep the limiter for the life
    /// of the app (in a static or app state): a memory limiter made per
    /// request forgets every attempt.
    #[cfg(feature = "throttle")]
    pub fn throttle<A: throttle::Algorithm, St: throttle::Store>(mut self, limiter: Arc<throttle::Limiter<A, St>>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn settings(&self) -> &Config {
        &self.cfg
    }

    pub async fn status(&self, user: i64) -> Result<Status, Error> {
        let secret = self.store.secret(user).await?;
        let enabled = secret.as_ref().is_some_and(|s| s.confirmed_at.is_some());
        let backup_codes_left = if enabled { self.store.unused_backup_codes(user).await? } else { 0 };
        Ok(Status { enabled, confirming: secret.is_some() && !enabled, backup_codes_left })
    }

    /// Whether logins for `user` need a code.
    pub async fn enabled(&self, user: i64) -> Result<bool, Error> {
        Ok(self.store.secret(user).await?.is_some_and(|s| s.confirmed_at.is_some()))
    }

    /// Start enrolment, or show the unconfirmed secret again (same QR on a
    /// reload). `account` labels it in the app, usually the email.
    /// [`Error::AlreadyEnabled`] once confirmed.
    pub async fn setup(&self, user: i64, account: &str) -> Result<Setup, Error> {
        let secret = match self.store.secret(user).await? {
            Some(s) if s.confirmed_at.is_some() => return Err(Error::AlreadyEnabled),
            Some(s) => self.open(user, &s.sealed)?,
            None => {
                let secret = random::<SECRET_BYTES>().to_vec();
                let sealed = self.crypt.encrypt(purpose(user).as_bytes(), &secret).map_err(Error::other)?;
                self.store.put_secret(user, &sealed, now()).await?;
                secret
            }
        };
        let encoded = Secret::Raw(secret).to_encoded().to_string();
        let label = format!("{}:{}", utf8_percent_encode(&self.issuer, NON_ALPHANUMERIC), utf8_percent_encode(account, NON_ALPHANUMERIC));
        let uri = format!(
            "otpauth://totp/{label}?secret={encoded}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP}",
            utf8_percent_encode(&self.issuer, NON_ALPHANUMERIC)
        );
        let qr_svg = qrcode::QrCode::new(uri.as_bytes())
            .map_err(Error::other)?
            .render::<qrcode::render::svg::Color<'_>>()
            .min_dimensions(200, 200)
            .build();
        Ok(Setup { secret: encoded, uri, qr_svg })
    }

    /// Turn two-factor on with a first valid code. Returns the backup codes:
    /// show them once, they are stored hashed.
    pub async fn confirm(&self, user: i64, code: &str) -> Result<Vec<String>, Error> {
        let stored = self.store.secret(user).await?.ok_or(Error::NotEnabled)?;
        if stored.confirmed_at.is_some() {
            return Err(Error::AlreadyEnabled);
        }
        let step = self.matching_step(user, &stored, code)?.ok_or(Error::InvalidCode)?;
        if !self.store.confirm_secret(user, now(), step).await? {
            return Err(Error::AlreadyEnabled);
        }
        self.new_backup_codes(user).await
    }

    /// Turn two-factor off: deletes the secret, backup codes and trusted
    /// browsers. Gate it behind `auth::PasswordConfirmed`.
    pub async fn disable(&self, user: i64) -> Result<(), Error> {
        self.store.delete_all(user).await
    }

    /// New backup codes; the old ones stop working. Gate it behind
    /// `auth::PasswordConfirmed`.
    pub async fn regenerate_backup_codes(&self, user: i64) -> Result<Vec<String>, Error> {
        if !self.enabled(user).await? {
            return Err(Error::NotEnabled);
        }
        self.new_backup_codes(user).await
    }

    async fn new_backup_codes(&self, user: i64) -> Result<Vec<String>, Error> {
        let codes: Vec<String> = (0..self.cfg.backup_codes).map(|_| backup_code()).collect();
        let hashes: Vec<Vec<u8>> = codes.iter().map(|c| hash_code(c)).collect();
        self.store.replace_backup_codes(user, &hashes).await?;
        Ok(codes)
    }

    fn open(&self, user: i64, sealed: &[u8]) -> Result<Vec<u8>, Error> {
        self.crypt.decrypt(purpose(user).as_bytes(), sealed).map_err(Error::other)
    }

    /// The time step `code` is valid for, within the skew window. Does not
    /// check replays.
    fn matching_step(&self, user: i64, stored: &StoredSecret, code: &str) -> Result<Option<i64>, Error> {
        let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        if code.len() != DIGITS || !code.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(None);
        }
        let totp = TOTP::new(Algorithm::SHA1, DIGITS, SKEW, STEP, self.open(user, &stored.sealed)?).map_err(Error::other)?;
        let current = now().max(0) as u64 / STEP;
        // Newest first, so a code valid in two steps records the later one.
        let window = (current.saturating_sub(SKEW as u64)..=current + SKEW as u64).rev();
        Ok(window.into_iter().find(|s| ct_eq(totp.generate(s * STEP).as_bytes(), code.as_bytes())).map(|s| s as i64))
    }

    async fn check_totp(&self, user: i64, code: &str) -> Result<bool, Error> {
        let Some(stored) = self.store.secret(user).await?.filter(|s| s.confirmed_at.is_some()) else { return Ok(false) };
        match self.matching_step(user, &stored, code)? {
            Some(step) => self.store.use_step(user, step).await,
            None => Ok(false),
        }
    }

    /// After a correct password for a user with two-factor on: remember the
    /// login in the session for `Config::pending_for`, instead of logging in.
    pub fn begin(&self, session: &Session, user: i64, remember: bool) -> Result<(), Error> {
        let pending = Pending { user_id: user, remember, expires_at: now() + self.cfg.pending_for.as_secs() as i64 };
        session.insert(PENDING_KEY, &pending).map_err(Error::other)
    }

    /// The user whose password was checked and who still needs a code.
    pub fn pending(&self, session: &Session) -> Option<i64> {
        self.pending_login(session).map(|p| p.user_id)
    }

    fn pending_login(&self, session: &Session) -> Option<Pending> {
        let pending = session.get::<Pending>(PENDING_KEY).ok().flatten()?;
        if pending.expires_at <= now() {
            session.remove(PENDING_KEY);
            return None;
        }
        Some(pending)
    }

    /// Check the code for the pending login. On success the pending login is
    /// gone: call `auth.login` (it rotates the session id).
    pub async fn challenge(&self, session: &Session, ip: Option<IpAddr>, code: Code<'_>) -> Result<Passed, ChallengeError> {
        let pending = self.pending_login(session).ok_or(ChallengeError::NoPending)?;
        let user = pending.user_id;
        let key = format!("two-factor:{user}|{}", ip.map(|ip| ip.to_string()).unwrap_or_default());
        let other = |e: BoxError| ChallengeError::Other(Error::Other(e));
        if let Some(l) = &self.limiter {
            if let Some(retry) = l.check(&key).await.map_err(other)? {
                return Err(ChallengeError::Throttled(retry));
            }
        }
        let backup_codes_left = match code {
            Code::Totp(c) => self.check_totp(user, c).await.map_err(ChallengeError::Other)?.then_some(None),
            Code::Backup(c) => {
                let used = self.store.use_backup_code(user, &hash_code(c), now()).await.map_err(ChallengeError::Other)?;
                match used {
                    true => Some(Some(self.store.unused_backup_codes(user).await.map_err(ChallengeError::Other)?)),
                    false => None,
                }
            }
        };
        let Some(backup_codes_left) = backup_codes_left else {
            if let Some(l) = &self.limiter {
                l.hit(&key).await.map_err(other)?;
            }
            return Err(ChallengeError::InvalidCode { user_id: user });
        };
        if let Some(l) = &self.limiter {
            l.clear(&key).await.map_err(other)?;
        }
        session.remove(PENDING_KEY);
        Ok(Passed { user_id: user, remember: pending.remember, backup_codes_left })
    }

    /// Finish the pending login with a second factor checked elsewhere (a
    /// passkey, verified for [`TwoFactor::pending`]'s user). Then call `auth.login`.
    pub fn pass(&self, session: &Session) -> Result<Passed, ChallengeError> {
        let pending = self.pending_login(session).ok_or(ChallengeError::NoPending)?;
        session.remove(PENDING_KEY);
        Ok(Passed { user_id: pending.user_id, remember: pending.remember, backup_codes_left: None })
    }

    /// Whether this browser's trusted cookie is valid for `user`: signed, not
    /// expired, and its device not revoked. Marks the device used.
    pub async fn trusted(&self, user: i64, device: &Device) -> Result<bool, Error> {
        let Some(cookie) = device.trust_cookie.as_deref() else { return Ok(false) };
        let Ok((owner, token)) = self.cookies.verify::<(i64, String)>(TRUST_COOKIE, cookie, self.cfg.trust_for) else { return Ok(false) };
        if owner != user {
            return Ok(false);
        }
        let Some(found) = self.store.device_by_token(&hash_token(&token)).await? else { return Ok(false) };
        if found.user_id != user || found.expires_at <= now() {
            return Ok(false);
        }
        self.store.touch_device(found.id, now()).await?;
        Ok(true)
    }

    /// Trust this browser for `Config::trust_for`. Returns the `Set-Cookie`
    /// header value to add to the response.
    pub async fn trust(&self, user: i64, device: &Device, ip: Option<IpAddr>) -> Result<String, Error> {
        let token = URL_SAFE_NO_PAD.encode(random::<32>());
        let created_at = now();
        self.store
            .add_device(NewDevice {
                uuid: Uuid::new(),
                user_id: user,
                token_hash: hash_token(&token),
                user_agent: device.user_agent.clone(),
                ip: ip.map(|ip| ip.to_string()),
                expires_at: created_at + self.cfg.trust_for.as_secs() as i64,
                created_at,
            })
            .await?;
        let value = self.cookies.sign(TRUST_COOKIE, &(user, token)).map_err(Error::other)?;
        Ok(self.cookie(&value, self.cfg.trust_for.as_secs()))
    }

    /// A `Set-Cookie` value that removes the trusted-browser cookie.
    pub fn forget_cookie(&self) -> String {
        self.cookie("", 0)
    }

    fn cookie(&self, value: &str, max_age: u64) -> String {
        let secure = if self.cfg.secure { "; Secure" } else { "" };
        format!("{TRUST_COOKIE}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}")
    }

    /// The user's trusted browsers, newest first.
    pub async fn devices(&self, user: i64) -> Result<Vec<TrustedDevice>, Error> {
        self.store.devices(user).await
    }

    /// Stop trusting one of the user's browsers. `false` if it is not theirs.
    pub async fn revoke(&self, user: i64, device: Uuid<TrustedDevice>) -> Result<bool, Error> {
        self.store.delete_device(user, device).await
    }
}

/// The browser the request came from: its trusted-browser cookie and user agent.
#[derive(Clone, Debug, Default)]
pub struct Device {
    pub trust_cookie: Option<String>,
    pub user_agent: Option<String>,
}

impl Device {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let trust_cookie = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .find(|(k, _)| *k == TRUST_COOKIE)
            .map(|(_, v)| v.to_owned())
            .filter(|v| !v.is_empty());
        // ponytail: user agents can be long; 512 chars is plenty to recognise a browser.
        let user_agent = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(|v| v.chars().take(512).collect());
        Self { trust_cookie, user_agent }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Device {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        Ok(Self::from_headers(&parts.headers))
    }
}

impl<S: Send + Sync> FromRequest<S> for Device {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}

/// The `crypt` purpose of a user's secret: a sealed secret only opens for its own row.
fn purpose(user: i64) -> String {
    format!("factor2/totp:{user}")
}

fn now() -> i64 {
    // A clock set before 1970 reads as 0 rather than panicking.
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    // No secure fallback exists: guessable secrets or codes must not be issued.
    #[allow(clippy::expect_used, reason = "the OS random source failed; there is no safe fallback")]
    getrandom::fill(&mut b).expect("OS random source");
    b
}

/// 32 letters and digits without look-alikes (`l`, `o`, `0`, `1`).
const ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";

/// `xxxxxxxxxx-xxxxxxxxxx`: 100 bits, like Fortify's two 10-character halves.
fn backup_code() -> String {
    let b = random::<20>();
    let mut s: String = b.iter().map(|x| ALPHABET[(x & 31) as usize] as char).collect();
    s.insert(10, '-');
    s
}

/// SHA-256 of a backup code, ignoring case, spaces and dashes. Unsalted is
/// fine: codes are random with 100 bits.
fn hash_code(code: &str) -> Vec<u8> {
    let normal: String = code.chars().filter(|c| !c.is_whitespace() && *c != '-').flat_map(char::to_lowercase).collect();
    Sha256::digest(normal.as_bytes()).to_vec()
}

/// SHA-256 of a trusted-browser token (256 random bits), exactly as issued.
fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_codes_look_like_fortify_and_hash_loosely() {
        let c = backup_code();
        assert_eq!(c.len(), 21);
        assert_eq!(c.as_bytes()[10], b'-');
        assert!(c.bytes().filter(|&b| b != b'-').all(|b| ALPHABET.contains(&b)));
        assert_eq!(hash_code(&c), hash_code(&c.to_uppercase().replace('-', " ")));
        assert_ne!(backup_code(), c);
    }
}
