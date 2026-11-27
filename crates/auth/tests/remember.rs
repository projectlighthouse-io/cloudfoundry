use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use auth::{Auth, Authed, NewToken, Provider, Remember, Tokens};
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
    /// hash -> (user id, token)
    tokens: Arc<Mutex<HashMap<String, (u32, NewToken)>>>,
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
    async fn by_token_hash(&self, hash: &str) -> Result<Option<(User, Option<i64>)>, auth::Error> {
        let t = self.tokens.lock().unwrap().get(hash).cloned();
        Ok(t.map(|(_, t)| (User { id: 1, name: "ada" }, t.expires_at)))
    }
    async fn store_token(&self, id: u32, token: NewToken) -> Result<(), auth::Error> {
        self.tokens.lock().unwrap().insert(token.hash.clone(), (id, token));
        Ok(())
    }
    async fn delete_token(&self, hash: &str) -> Result<(), auth::Error> {
        self.tokens.lock().unwrap().remove(hash);
        Ok(())
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
    let u = users.clone();
    r.post("/api/logout", move |auth: Auth<User>| {
        let u = u.clone();
        async move {
            auth::revoke_token(&u, auth.bearer_token().unwrap()).await.unwrap();
            "revoked"
        }
    });
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

fn bearer(token: &str) -> Option<(&'static str, &'static str)> {
    Some(("authorization", String::leak(format!("Bearer {token}"))))
}

#[tokio::test]
async fn bearer_tokens() {
    let users = Users::default();
    let r = app(users.clone());
    let ada = User { id: 1, name: "ada" };
    let token = auth::issue_token(&users, &ada, "cli", None).await.unwrap();
    let stored = users.tokens.lock().unwrap().values().next().unwrap().1.clone();
    assert_eq!((stored.name.as_str(), stored.expires_at), ("cli", None));
    assert_eq!(stored.hash, auth::hash_token(&token));
    assert_ne!(stored.hash, token, "only the hash is stored");

    let (status, body, set) = send(&r, Method::GET, "/me", &[], bearer(&token)).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "ada"));
    assert!(set.is_empty(), "no session for API calls");
    assert_eq!(send(&r, Method::GET, "/me", &[], bearer("nope")).await.0, StatusCode::UNAUTHORIZED);

    assert_eq!(send(&r, Method::POST, "/api/logout", &[], bearer(&token)).await.1, "revoked");
    assert_eq!(send(&r, Method::GET, "/me", &[], bearer(&token)).await.0, StatusCode::UNAUTHORIZED, "revoked token");
}

#[tokio::test]
async fn expired_tokens_are_rejected() {
    let users = Users::default();
    let r = app(users.clone());
    let ada = User { id: 1, name: "ada" };
    let live = auth::issue_token(&users, &ada, "live", Some(std::time::Duration::from_secs(3600))).await.unwrap();
    assert_eq!(send(&r, Method::GET, "/me", &[], bearer(&live)).await.1, "ada");

    let old = "old-token";
    users.tokens.lock().unwrap().insert(auth::hash_token(old), (1, NewToken { name: "old".into(), hash: auth::hash_token(old), expires_at: Some(1) }));
    assert_eq!(send(&r, Method::GET, "/me", &[], bearer(old)).await.0, StatusCode::UNAUTHORIZED);
}
