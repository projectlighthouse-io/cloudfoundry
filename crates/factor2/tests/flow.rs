#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
//! The login flow as the published handlers run it: `auth.validate`, then
//! either `auth.login` or `TwoFactor::begin`, then `TwoFactor::challenge`.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use auth::{Auth, Authed, Credentials, Provider};
use factor2::{ChallengeError, Code, Device, MemoryStore, TwoFactor};
use http_body_util::BodyExt;
use route::{
    IntoResponse, Path, Router, Routes, empty,
    http::{HeaderValue, Method, Request, header},
};
use securecookie::{Codec, Keys};
use session::Session;
use totp_rs::{Algorithm, Secret, TOTP};

const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

#[derive(Clone)]
struct User {
    id: i64,
    hash: String,
}

#[derive(Clone)]
struct Users(Arc<Vec<User>>);

impl Provider for Users {
    type User = User;
    type Id = i64;
    async fn find(&self, id: i64) -> Result<Option<User>, auth::Error> {
        Ok(self.0.iter().find(|u| u.id == id).cloned())
    }
    fn id(&self, user: &User) -> i64 {
        user.id
    }
}

impl Credentials for Users {
    async fn by_login(&self, login: &str) -> Result<Option<User>, auth::Error> {
        Ok((login == "ada").then(|| self.0[0].clone()))
    }
    fn password_hash(&self, user: &User) -> Option<String> {
        Some(user.hash.clone())
    }
    async fn set_password_hash(&self, _: i64, _: String) -> Result<(), auth::Error> {
        Ok(())
    }
}

type Tf = TwoFactor<MemoryStore>;

fn two_factor(cfg: factor2::Config) -> Tf {
    let crypt = crypt::Encrypter::new(crypt::Keys::parse(&crypt::Keys::generate()).unwrap());
    TwoFactor::new(MemoryStore::new(), crypt, Codec::new(Keys::parse(KEY).unwrap()), "Acme Inc").config(cfg).unwrap()
}

#[derive(serde::Deserialize)]
struct Challenge {
    kind: String,
    code: String,
    trust: bool,
}

fn app(tf: Tf) -> Routes {
    let users = Users(Arc::new(vec![User { id: 1, hash: password::hash("secret").unwrap() }]));
    let mut r = Router::new();
    let (u, t) = (users.clone(), tf.clone());
    // The published login handler, minus validation and redirects.
    r.post("/login", move |(auth, session, device): (Auth<User>, Session, Device)| {
        let (u, t) = (u.clone(), t.clone());
        async move {
            let user = auth.validate(&u, "ada", "secret").await.unwrap();
            if t.enabled(user.id).await.unwrap() && !t.trusted(user.id, &device).await.unwrap() {
                t.begin(&session, user.id, false).unwrap();
                return "challenge";
            }
            auth.login(&user);
            "in"
        }
    });
    let (u, t) = (users.clone(), tf);
    r.post("/challenge/{kind}/{code}/{trust}", move |(auth, session, device, Path(c)): (Auth<User>, Session, Device, Path<Challenge>)| {
        let (u, t) = (u.clone(), t.clone());
        async move {
            let code = if c.kind == "totp" { Code::Totp(&c.code) } else { Code::Backup(&c.code) };
            match t.challenge(&session, None, code).await {
                Ok(passed) => {
                    let user = u.find(passed.user_id).await.unwrap().unwrap();
                    auth.login(&user);
                    let mut res = "in".into_response();
                    if c.trust {
                        let cookie = t.trust(user.id, &device, None).await.unwrap();
                        res.headers_mut().append(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
                    }
                    res
                }
                Err(ChallengeError::NoPending) => "no pending".into_response(),
                Err(ChallengeError::InvalidCode { .. }) => "invalid".into_response(),
                Err(ChallengeError::Throttled(_)) => "throttled".into_response(),
                Err(e) => panic!("{e}"),
            }
        }
    });
    r.post("/logout", |auth: Auth<User>| async move {
        auth.logout();
        "out"
    });
    r.get("/me", |_: Authed<User>| async { "me" });
    r.layer(session::layer(session::MemoryStore::new(), Codec::new(Keys::parse(KEY).unwrap()))).layer(auth::layer(users));
    r.build(()).unwrap()
}

/// A browser: keeps cookies between requests.
struct Browser {
    routes: Routes,
    jar: BTreeMap<String, String>,
}

impl Browser {
    async fn send(&mut self, method: Method, uri: &str) -> String {
        let mut req = Request::builder().method(method).uri(uri).header(header::ACCEPT, "application/json");
        let cookies: Vec<String> = self.jar.iter().map(|(k, v)| format!("{k}={v}")).collect();
        if !cookies.is_empty() {
            req = req.header(header::COOKIE, cookies.join("; "));
        }
        let res = self.routes.handle(req.body(empty()).unwrap()).await;
        for set in res.headers().get_all(header::SET_COOKIE) {
            let pair = set.to_str().unwrap().split(';').next().unwrap();
            let (k, v) = pair.split_once('=').unwrap();
            if v.is_empty() { self.jar.remove(k) } else { self.jar.insert(k.into(), v.into()) };
        }
        let status = res.status();
        let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        if status.is_success() { body } else { status.as_str().to_owned() }
    }

    async fn post(&mut self, uri: &str) -> String {
        self.send(Method::POST, uri).await
    }

    async fn me(&mut self) -> String {
        self.send(Method::GET, "/me").await
    }
}

fn unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// The code for `offset` steps from now, from the base32 secret the setup showed.
fn code(secret: &str, offset: i64) -> String {
    let totp = TOTP::new(Algorithm::SHA1, 6, 1, 30, Secret::Encoded(secret.into()).to_bytes().unwrap()).unwrap();
    totp.generate((unix() as i64 + offset * 30) as u64)
}

/// A six-digit code that is valid for no step in the window.
fn wrong(secret: &str) -> String {
    let valid: Vec<String> = (-1..=1).map(|o| code(secret, o)).collect();
    (0..1000).map(|n| format!("{n:06}")).find(|c| !valid.contains(c)).unwrap()
}

/// Codes from `code` must stay in the ±1 window for the whole test: start
/// well inside a 30 s step.
async fn away_from_step_edge() {
    while unix() % 30 > 20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn second_factor_guards_the_login() {
    away_from_step_edge().await;
    let tf = two_factor(factor2::Config::default());
    let mut b = Browser { routes: app(tf.clone()), jar: BTreeMap::new() };

    // Without two-factor, the password is enough.
    assert_eq!(b.post("/login").await, "in");
    b.post("/logout").await;

    // Enrol: the setup is stable until confirmed; a wrong code does not confirm.
    let setup = tf.setup(1, "ada@example.com").await.unwrap();
    assert!(setup.uri.starts_with("otpauth://totp/Acme%20Inc:ada%40example%2Ecom?secret="), "{}", setup.uri);
    assert!(setup.qr_svg.contains("<svg"));
    assert_eq!(tf.setup(1, "ada@example.com").await.unwrap().secret, setup.secret, "same secret on reload");
    assert!(!tf.enabled(1).await.unwrap(), "unconfirmed: logins do not ask yet");
    assert!(matches!(tf.confirm(1, &wrong(&setup.secret)).await, Err(factor2::Error::InvalidCode)));
    let backup = tf.confirm(1, &code(&setup.secret, -1)).await.unwrap();
    assert_eq!(backup.len(), 10);
    assert!(matches!(tf.setup(1, "ada@example.com").await, Err(factor2::Error::AlreadyEnabled)));

    // Login blocked without a code.
    assert_eq!(b.post("/login").await, "challenge");
    assert_eq!(b.me().await, "401", "a correct password alone does not log in");

    // Wrong code.
    assert_eq!(b.post(&format!("/challenge/totp/{}/false", wrong(&setup.secret))).await, "invalid");
    assert_eq!(b.me().await, "401");
    assert_eq!(b.post(&format!("/challenge/totp/{}/false", code(&setup.secret, -1))).await, "invalid", "the confirming code's step is used");

    // Right code logs in; the session id rotates.
    let before = b.jar.get("session").cloned();
    assert_eq!(b.post(&format!("/challenge/totp/{}/false", code(&setup.secret, 0))).await, "in");
    assert_ne!(b.jar.get("session").cloned(), before, "session id rotated");
    assert_eq!(b.me().await, "me");
    assert_eq!(b.post(&format!("/challenge/totp/{}/false", code(&setup.secret, 1))).await, "no pending", "nothing waits any more");

    // Replayed code.
    b.post("/logout").await;
    assert_eq!(b.post("/login").await, "challenge");
    assert_eq!(b.post(&format!("/challenge/totp/{}/false", code(&setup.secret, 0))).await, "invalid", "same code twice");
    assert_eq!(b.post(&format!("/challenge/totp/{}/false", code(&setup.secret, 1))).await, "in", "next step, within the window");

    // Backup codes work once.
    b.post("/logout").await;
    assert_eq!(b.post("/login").await, "challenge");
    assert_eq!(b.post(&format!("/challenge/backup/{}/false", backup[0].to_uppercase())).await, "in", "case-insensitive");
    assert_eq!(tf.status(1).await.unwrap().backup_codes_left, 9);
    b.post("/logout").await;
    assert_eq!(b.post("/login").await, "challenge");
    assert_eq!(b.post(&format!("/challenge/backup/{}/false", backup[0])).await, "invalid", "used up");

    // Trusted browser skips the challenge, until revoked.
    assert_eq!(b.post(&format!("/challenge/backup/{}/true", backup[1])).await, "in");
    assert!(b.jar.contains_key(factor2::TRUST_COOKIE));
    b.post("/logout").await;
    assert_eq!(b.post("/login").await, "in", "trusted: no challenge");
    assert_eq!(b.me().await, "me");
    let devices = tf.devices(1).await.unwrap();
    assert_eq!(devices.len(), 1);
    assert!(devices[0].last_used_at.is_some());
    let json = serde_json::to_value(&devices[0]).unwrap();
    assert!(json.get("id").is_none() && json.get("user_id").is_none() && json.get("uuid").is_some(), "{json}");

    // Another user's or a forged cookie does not count.
    let mut other = Browser { routes: b.routes.clone(), jar: BTreeMap::new() };
    other.jar.insert(factor2::TRUST_COOKIE.into(), "forged".into());
    assert_eq!(other.post("/login").await, "challenge");

    assert!(tf.revoke(1, devices[0].uuid).await.unwrap());
    b.post("/logout").await;
    assert_eq!(b.post("/login").await, "challenge", "revoked");

    // Regenerating replaces the codes.
    let fresh = tf.regenerate_backup_codes(1).await.unwrap();
    assert_eq!(b.post(&format!("/challenge/backup/{}/false", backup[2])).await, "invalid", "old codes stop working");
    assert_eq!(b.post(&format!("/challenge/backup/{}/false", fresh[0])).await, "in");

    // Disabling removes it all.
    tf.disable(1).await.unwrap();
    b.post("/logout").await;
    assert_eq!(b.post("/login").await, "in");
    assert!(tf.devices(1).await.unwrap().is_empty());
    assert_eq!(tf.status(1).await.unwrap(), factor2::Status { enabled: false, confirming: false, backup_codes_left: 0 });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pending_login_expires() {
    let tf = two_factor(factor2::Config { pending_for: Duration::from_secs(1), ..Default::default() });
    let mut b = Browser { routes: app(tf.clone()), jar: BTreeMap::new() };
    let setup = tf.setup(1, "ada").await.unwrap();
    let backup = tf.confirm(1, &code(&setup.secret, 0)).await.unwrap();
    assert_eq!(b.post(&format!("/challenge/backup/{}/false", backup[0])).await, "no pending", "no password checked");
    assert_eq!(b.post("/login").await, "challenge");
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(b.post(&format!("/challenge/backup/{}/false", backup[0])).await, "no pending", "expired: log in again");
}

#[cfg(feature = "throttle")]
#[tokio::test(flavor = "multi_thread")]
async fn wrong_codes_are_throttled() {
    let limiter = Arc::new(throttle::Limiter::memory(throttle::FixedWindow::new(3, Duration::from_secs(60))));
    let tf = two_factor(factor2::Config::default()).throttle(limiter);
    let mut b = Browser { routes: app(tf.clone()), jar: BTreeMap::new() };
    let setup = tf.setup(1, "ada").await.unwrap();
    let backup = tf.confirm(1, &code(&setup.secret, 0)).await.unwrap();
    assert_eq!(b.post("/login").await, "challenge");
    for _ in 0..3 {
        assert_eq!(b.post("/challenge/backup/nope/false").await, "invalid");
    }
    assert_eq!(b.post(&format!("/challenge/backup/{}/false", backup[0])).await, "throttled", "even a right code waits");
}

#[test]
fn pass_finishes_the_pending_login_once() {
    let tf = two_factor(factor2::Config::default());
    let session = Session::new();
    assert!(matches!(tf.pass(&session), Err(ChallengeError::NoPending)));
    tf.begin(&session, 1, true).unwrap();
    let passed = tf.pass(&session).unwrap();
    assert_eq!((passed.user_id, passed.remember, passed.backup_codes_left), (1, true, None));
    assert_eq!(tf.pending(&session), None);
    assert!(matches!(tf.pass(&session), Err(ChallengeError::NoPending)));
}
