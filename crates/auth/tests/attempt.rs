#![cfg(feature = "throttle")]

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use auth::{AttemptError, Auth, Authed, Credentials, Provider};
use http_body_util::BodyExt;
use route::{
    Path, Router, Routes, empty,
    http::{Request, header},
};
use securecookie::{Codec, Keys};
use throttle::{FixedWindow, Limiter};

#[derive(serde::Deserialize)]
struct Creds {
    email: String,
    pw: String,
}

#[derive(Clone)]
struct User {
    id: u32,
    email: &'static str,
    hash: String,
}

#[derive(Clone)]
struct Users(Arc<Mutex<Vec<User>>>);

impl Provider for Users {
    type User = User;
    type Id = u32;

    async fn find(&self, id: u32) -> Result<Option<User>, auth::Error> {
        Ok(self.0.lock().unwrap().iter().find(|u| u.id == id).cloned())
    }

    fn id(&self, user: &User) -> u32 {
        user.id
    }
}

impl Credentials for Users {
    async fn by_login(&self, login: &str) -> Result<Option<User>, auth::Error> {
        Ok(self.0.lock().unwrap().iter().find(|u| u.email.eq_ignore_ascii_case(login)).cloned())
    }

    /// An empty hash stands for an account made through social login.
    fn password_hash(&self, user: &User) -> Option<String> {
        Some(user.hash.clone()).filter(|h| !h.is_empty())
    }

    async fn set_password_hash(&self, id: u32, hash: String) -> Result<(), auth::Error> {
        self.0.lock().unwrap().iter_mut().find(|u| u.id == id).unwrap().hash = hash;
        Ok(())
    }
}

/// A valid hash made with weaker parameters than `password::hash` uses.
fn old_hash(pw: &str) -> String {
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version, password_hash::SaltString};
    let a = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::new(8192, 1, 1, None).unwrap());
    a.hash_password(pw.as_bytes(), &SaltString::encode_b64(b"0123456789abcdef").unwrap()).unwrap().to_string()
}

fn app(users: Users) -> Routes {
    let mut r = Router::new();
    let u = users.clone();
    r.post("/login/{email}/{pw}", move |(auth, Path(Creds { email, pw })): (Auth<User>, Path<Creds>)| {
        let u = u.clone();
        async move {
            match auth.attempt(&u, &email, &pw, false).await {
                Ok(user) => format!("in {}", user.id),
                Err(AttemptError::Failed) => "failed".into(),
                Err(AttemptError::Throttled(d)) => format!("throttled {}", d.as_secs()),
                Err(AttemptError::Other(e)) => format!("error {e}"),
            }
        }
    });
    r.get("/me", |u: Authed<User>| async move { u.email });
    r.layer(session::layer(session::MemoryStore::new(), Codec::new(Keys::parse(&Keys::generate()).unwrap())))
        .layer(auth::layer(users).throttle(Limiter::memory(FixedWindow::new(2, Duration::from_secs(60)))));
    r.build(()).unwrap()
}

async fn send(r: &Routes, jar: &mut Option<String>, method: &str, uri: &str, ip: [u8; 4]) -> String {
    let mut req = Request::builder().method(method).uri(uri).header(header::ACCEPT, "application/json");
    if let Some(c) = jar.as_ref() {
        req = req.header(header::COOKIE, c.as_str());
    }
    let mut req = req.body(empty()).unwrap();
    req.extensions_mut().insert(route::ClientAddr((ip, 1).into()));
    let res = r.handle(req).await;
    if let Some(c) = res.headers().get(header::SET_COOKIE) {
        *jar = Some(c.to_str().unwrap().split(';').next().unwrap().to_owned());
    }
    let status = res.status();
    let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    if status.is_success() { body } else { status.as_str().to_owned() }
}

fn users() -> Users {
    Users(Arc::new(Mutex::new(vec![
        User { id: 1, email: "ada@example.com", hash: password::hash("right").unwrap() },
        User { id: 2, email: "bob@example.com", hash: old_hash("bobpw") },
        User { id: 3, email: "social@example.com", hash: String::new() },
    ])))
}

#[tokio::test(flavor = "multi_thread")]
async fn attempt_logs_in_only_with_the_right_password() {
    let r = app(users());
    let jar = &mut None;
    let ip = [10, 0, 0, 1];
    assert_eq!(send(&r, jar, "POST", "/login/ada@example.com/wrong", ip).await, "failed");
    assert_eq!(send(&r, jar, "GET", "/me", ip).await, "401");
    assert_eq!(send(&r, jar, "POST", "/login/ADA@example.com/right", ip).await, "in 1", "login is case-insensitive here");
    assert_eq!(send(&r, jar, "GET", "/me", ip).await, "ada@example.com");
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_login_fails_the_same_way() {
    let r = app(users());
    assert_eq!(send(&r, &mut None, "POST", "/login/nobody@example.com/x", [10, 0, 0, 1]).await, "failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn failures_are_throttled_per_login_and_ip() {
    let r = app(users());
    let jar = &mut None;
    let ip = [10, 0, 0, 1];
    assert_eq!(send(&r, jar, "POST", "/login/ada@example.com/a", ip).await, "failed");
    assert_eq!(send(&r, jar, "POST", "/login/ada@example.com/b", ip).await, "failed");
    let third = send(&r, jar, "POST", "/login/ada@example.com/right", ip).await;
    assert!(third.starts_with("throttled 5") || third.starts_with("throttled 6"), "even the right password waits: {third}");
    assert_eq!(send(&r, jar, "POST", "/login/Ada@Example.com/right", ip).await.split(' ').next(), Some("throttled"), "key ignores case");
    assert_eq!(send(&r, jar, "POST", "/login/ada@example.com/right", [10, 0, 0, 2]).await, "in 1", "another IP is not blocked");
    assert_eq!(send(&r, jar, "POST", "/login/bob@example.com/x", ip).await, "failed", "another login is not blocked");
}

#[tokio::test(flavor = "multi_thread")]
async fn success_clears_failures() {
    let r = app(users());
    let jar = &mut None;
    let ip = [10, 0, 0, 1];
    assert_eq!(send(&r, jar, "POST", "/login/ada@example.com/a", ip).await, "failed");
    assert_eq!(send(&r, jar, "POST", "/login/ada@example.com/right", ip).await, "in 1");
    assert_eq!(send(&r, jar, "POST", "/login/ada@example.com/b", ip).await, "failed");
    assert_eq!(send(&r, jar, "POST", "/login/ada@example.com/c", ip).await, "failed", "count restarted after the success");
}

#[tokio::test(flavor = "multi_thread")]
async fn old_hashes_are_upgraded_on_login() {
    let users = users();
    let r = app(users.clone());
    let before = users.0.lock().unwrap()[1].hash.clone();
    assert!(password::needs_rehash(&before));
    assert_eq!(send(&r, &mut None, "POST", "/login/bob@example.com/bobpw", [10, 0, 0, 1]).await, "in 2");
    let after = users.0.lock().unwrap()[1].hash.clone();
    assert_ne!(after, before);
    assert!(!password::needs_rehash(&after));
    assert!(password::verify("bobpw", &after).is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn hash_password_makes_a_current_hash() {
    let h = auth::hash_password("secret").await.unwrap();
    assert!(password::verify("secret", &h).is_ok());
    assert!(!password::needs_rehash(&h));
}

#[tokio::test(flavor = "multi_thread")]
async fn accounts_without_a_password_cannot_log_in_with_one() {
    let r = app(users());
    for pw in ["", "anything"] {
        assert_eq!(send(&r, &mut None, "POST", &format!("/login/social@example.com/{pw}x"), [10, 0, 0, 9]).await, "failed");
    }
}
