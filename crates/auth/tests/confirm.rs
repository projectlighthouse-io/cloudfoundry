#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
//! Sudo mode (`confirm_password` + `PasswordConfirmed`) and `validate`.
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use auth::{Auth, Authed, ChangeError, Credentials, PasswordConfirmed, Provider};
use http_body_util::BodyExt;
use route::{
    Path, Router, Routes, empty,
    http::{Method, Request, StatusCode, header},
};
use securecookie::{Codec, Keys};

#[derive(Clone)]
struct User {
    id: u32,
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
        Ok((login == "ada").then(|| self.0.lock().unwrap()[0].clone()))
    }
    fn password_hash(&self, user: &User) -> Option<String> {
        Some(user.hash.clone())
    }
    async fn set_password_hash(&self, _: u32, _: String) -> Result<(), auth::Error> {
        Ok(())
    }
}

fn app(timeout: Duration) -> Routes {
    let users = Users(Arc::new(Mutex::new(vec![User { id: 1, hash: password::hash("secret").unwrap() }])));
    let codec = Codec::new(Keys::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap());
    let cfg = auth::Config { password_timeout: timeout, ..auth::Config::default() };
    let mut r = Router::new();
    let u = users.clone();
    r.post("/validate", move |auth: Auth<User>| {
        let u = u.clone();
        async move { auth.validate(&u, "ada", "secret").await.map(|u| format!("valid {}", u.id)).unwrap_or_default() }
    });
    let u = users.clone();
    r.post("/login", move |auth: Auth<User>| {
        let u = u.clone();
        async move { auth.attempt(&u, "ada", "secret", false).await.map(|_| "in").unwrap_or("failed") }
    });
    let u = users.clone();
    r.post("/confirm/{pw}", move |(auth, Path(pw)): (Auth<User>, Path<String>)| {
        let u = u.clone();
        async move {
            match auth.confirm_password(&u, &pw).await {
                Ok(()) => auth.intended("/home"),
                Err(ChangeError::WrongPassword) => "wrong".into(),
                Err(e) => e.to_string(),
            }
        }
    });
    r.get("/confirm-password", |_: ()| async { "confirm page" }).name("password.confirm");
    r.get("/danger", |_: PasswordConfirmed<User>| async { "danger zone" });
    r.delete("/danger", |_: PasswordConfirmed<User>| async { "deleted" });
    r.get("/me", |_: Authed<User>| async { "ada" });
    r.layer(session::layer(session::MemoryStore::new(), codec)).layer(auth::layer(users).config(cfg).unwrap());
    r.build(()).unwrap()
}

/// Body for 200, `"<status> <location>"` otherwise, and the session cookie if set.
async fn send(r: &Routes, method: Method, uri: &str, cookie: Option<&str>, json: bool) -> (String, Option<String>) {
    send_from(r, method, uri, cookie, json, None).await
}

async fn send_from(r: &Routes, method: Method, uri: &str, cookie: Option<&str>, json: bool, referer: Option<&str>) -> (String, Option<String>) {
    let mut req = Request::builder().method(method).uri(uri).header(header::HOST, "app.test");
    if let Some(r) = referer {
        req = req.header(header::REFERER, r);
    }
    if json {
        req = req.header(header::ACCEPT, "application/json");
    }
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let res = r.handle(req.body(empty()).unwrap()).await;
    let set = res.headers().get(header::SET_COOKIE).map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned());
    let status = res.status();
    let location = res.headers().get(header::LOCATION).map(|v| v.to_str().unwrap().to_owned()).unwrap_or_default();
    let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    (if status == StatusCode::OK { body } else { format!("{} {location}", status.as_u16()).trim().to_owned() }, set)
}

#[tokio::test(flavor = "multi_thread")]
async fn validate_checks_the_password_without_logging_in() {
    let r = app(Duration::from_secs(60));
    let (body, cookie) = send(&r, Method::POST, "/validate", None, true).await;
    assert_eq!(body, "valid 1");
    let cookie = cookie.unwrap_or_default();
    assert_eq!(send(&r, Method::GET, "/me", Some(&cookie), true).await.0, "401", "not logged in");
}

#[tokio::test(flavor = "multi_thread")]
async fn sudo_mode_needs_a_recent_confirmation() {
    let r = app(Duration::from_secs(1));
    assert_eq!(send(&r, Method::GET, "/danger", None, true).await.0, "401", "anonymous");
    let session = send(&r, Method::POST, "/login", None, true).await.1.unwrap();

    assert_eq!(send(&r, Method::GET, "/danger", Some(&session), true).await.0, "423", "JSON: locked");
    let (page, _) = send(&r, Method::GET, "/danger?tab=keys", Some(&session), false).await;
    assert_eq!(page, "303 /confirm-password", "page visit: to the confirm page");

    assert_eq!(send(&r, Method::POST, "/confirm/nope", Some(&session), true).await.0, "wrong");
    assert_eq!(send(&r, Method::POST, "/confirm/secret", Some(&session), true).await.0, "/danger?tab=keys", "back to the intended page");
    assert_eq!(send(&r, Method::GET, "/danger", Some(&session), true).await.0, "danger zone");
    assert_eq!(send(&r, Method::POST, "/confirm/secret", Some(&session), true).await.0, "/home", "intended is taken once");

    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(send(&r, Method::GET, "/danger", Some(&session), true).await.0, "423", "expired");

    // A form submission returns to the page it came from, never off-site.
    send_from(&r, Method::DELETE, "/danger", Some(&session), false, Some("https://evil.test/phish")).await;
    assert_eq!(send(&r, Method::POST, "/confirm/secret", Some(&session), true).await.0, "/danger", "foreign referer ignored: the expired GET above is still the intended page");
    tokio::time::sleep(Duration::from_millis(1100)).await;
    send_from(&r, Method::DELETE, "/danger", Some(&session), false, Some("https://app.test/settings/two-factor")).await;
    assert_eq!(send(&r, Method::POST, "/confirm/secret", Some(&session), true).await.0, "/settings/two-factor");
}
