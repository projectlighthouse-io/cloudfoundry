use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use auth::{Auth, Authed, Provider, Remember, Tokens};
use http_body_util::BodyExt;
use route::{
    Router, Routes, empty,
    http::{Method, Request, StatusCode, header},
};
use securecookie::{Codec, Keys};
use session::MemoryStore;

#[derive(Clone)]
struct User {
    id: u32,
    name: &'static str,
}

#[derive(Clone, Default)]
struct Users {
    remember: Arc<Mutex<HashMap<u32, String>>>,
}

impl Provider for Users {
    type User = User;
    type Id = u32;
    async fn find(&self, id: u32) -> Result<Option<User>, auth::Error> {
        Ok((id == 1).then_some(User { id: 1, name: "ada" }))
    }
    fn id(&self, user: &User) -> u32 {
        user.id
    }
}

impl Remember for Users {
    fn remember_token(&self, user: &User) -> Option<String> {
        self.remember.lock().unwrap().get(&user.id).cloned()
    }
    async fn set_remember_token(&self, id: u32, token: Option<String>) -> Result<(), auth::Error> {
        let mut m = self.remember.lock().unwrap();
        match token {
            Some(t) => m.insert(id, t),
            None => m.remove(&id),
        };
        Ok(())
    }
}

impl Tokens for Users {
    async fn by_token(&self, token: &str) -> Result<Option<User>, auth::Error> {
        Ok((token == "api-secret").then_some(User { id: 1, name: "ada" }))
    }
}

fn app(users: Users) -> Routes {
    let codec = || Codec::new(Keys::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap());
    let mut r = Router::new();
    r.post("/login", |auth: Auth<User>| async move {
        auth.login_remember(&User { id: 1, name: "ada" });
        "in"
    });
    r.post("/logout", |auth: Auth<User>| async move {
        auth.logout();
        "out"
    });
    r.get("/me", |u: Authed<User>| async move { u.name });
    r.layer(session::layer(MemoryStore::new(), codec())).layer(auth::layer(users).remember(codec()).tokens());
    r.build(()).unwrap()
}

/// Send with the given cookies; return status, body and the Set-Cookie pairs.
async fn send(r: &Routes, method: Method, uri: &str, cookies: &[String], extra: Option<(&str, &str)>) -> (StatusCode, String, Vec<String>) {
    let mut req = Request::builder().method(method).uri(uri).header(header::ACCEPT, "application/json");
    if !cookies.is_empty() {
        req = req.header(header::COOKIE, cookies.join("; "));
    }
    if let Some((k, v)) = extra {
        req = req.header(k, v);
    }
    let res = r.handle(req.body(empty()).unwrap()).await;
    let set: Vec<String> =
        res.headers().get_all(header::SET_COOKIE).iter().map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned()).collect();
    let status = res.status();
    (status, String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap(), set)
}

fn named<'a>(set: &'a [String], name: &str) -> Option<&'a String> {
    set.iter().find(|c| c.starts_with(&format!("{name}=")))
}

#[tokio::test]
async fn remember_me_survives_the_session_and_dies_on_logout() {
    let users = Users::default();
    let r = app(users.clone());

    let (_, _, set) = send(&r, Method::POST, "/login", &[], None).await;
    let remember = named(&set, "remember").expect("remember cookie set").clone();
    assert!(users.remember.lock().unwrap().contains_key(&1), "token stored");

    // A new browser session with only the remember cookie: logged back in, new session issued.
    let (status, body, set) = send(&r, Method::GET, "/me", &[remember.clone()], None).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "ada"));
    let session = named(&set, "session").expect("session re-established").clone();
    assert_eq!(send(&r, Method::GET, "/me", &[session.clone()], None).await.1, "ada");

    // Logout clears the stored token and the cookie.
    let (_, _, set) = send(&r, Method::POST, "/logout", &[session, remember.clone()], None).await;
    assert_eq!(named(&set, "remember").map(String::as_str), Some("remember="), "cookie expired");
    assert!(users.remember.lock().unwrap().is_empty(), "token cleared");
    assert_eq!(send(&r, Method::GET, "/me", &[remember], None).await.0, StatusCode::UNAUTHORIZED, "old cookie is dead");

    // A forged cookie is rejected and expired.
    let (status, _, set) = send(&r, Method::GET, "/me", &["remember=forged".into()], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(named(&set, "remember").map(String::as_str), Some("remember="));
}

#[tokio::test]
async fn bearer_tokens() {
    let r = app(Users::default());
    let (status, body, set) = send(&r, Method::GET, "/me", &[], Some(("authorization", "Bearer api-secret"))).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "ada"));
    assert!(set.is_empty(), "no session for API calls");
    assert_eq!(send(&r, Method::GET, "/me", &[], Some(("authorization", "Bearer nope"))).await.0, StatusCode::UNAUTHORIZED);
}
