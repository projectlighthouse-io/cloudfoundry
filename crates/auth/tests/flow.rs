#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
use auth::{Auth, Authed, Provider};
use http_body_util::BodyExt;
use route::{
    Path, Router, Routes, empty,
    http::{Method, Request, StatusCode, header},
};
use securecookie::{Codec, Keys};
use session::MemoryStore;

#[derive(Clone)]
struct User {
    id: u32,
    name: &'static str,
}

struct Users;

impl Provider for Users {
    type User = User;
    type Id = u32;

    async fn find(&self, id: u32) -> Result<Option<User>, auth::Error> {
        Ok([User { id: 1, name: "ada" }].into_iter().find(|u| u.id == id))
    }

    fn id(&self, user: &User) -> u32 {
        user.id
    }
}

async fn login(auth: Auth<User>, Path(id): Path<u32>) -> &'static str {
    auth.login(&User { id, name: "" });
    "in"
}
async fn logout(auth: Auth<User>) -> &'static str {
    auth.logout();
    "out"
}
async fn me(user: Authed<User>) -> &'static str {
    user.name
}

fn routes() -> Routes {
    let mut r = Router::new();
    r.get("/", |_: ()| async { "home" }).name("home");
    r.get("/login", |_: ()| async { "form" }).name("login");
    r.post("/login/{id}", login);
    r.post("/logout", logout);
    r.get("/me", me);
    r.group("/private", |r| {
        r.layer(auth::require::<User>());
        r.get("/", |_: ()| async { "secret" });
    });
    r.group("/guest", |r| {
        r.layer(auth::guest::<User>());
        r.get("/", |_: ()| async { "welcome" });
    });
    r.layer(session::layer(MemoryStore::new(), Codec::new(Keys::parse(&Keys::generate()).unwrap())))
        .layer(auth::layer(Users));
    r.build(()).unwrap()
}

async fn send(r: &Routes, jar: &mut Option<String>, method: Method, uri: &str, json: bool) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(c) = jar.as_ref() {
        req = req.header(header::COOKIE, c.as_str());
    }
    if json {
        req = req.header(header::ACCEPT, "application/json");
    }
    let res = r.handle(req.body(empty()).unwrap()).await;
    if let Some(c) = res.headers().get(header::SET_COOKIE) {
        *jar = Some(c.to_str().unwrap().split(';').next().unwrap().to_owned());
    }
    let status = res.status();
    let loc = res.headers().get(header::LOCATION).map(|v| v.to_str().unwrap().to_owned());
    let text = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    (status, loc.unwrap_or(text))
}

#[tokio::test]
async fn login_guard_logout() {
    let r = routes();
    let jar = &mut None;

    assert_eq!(send(&r, jar, Method::GET, "/me", false).await, (StatusCode::SEE_OTHER, "/login".into()), "page visit redirects");
    assert_eq!(send(&r, jar, Method::GET, "/me", true).await.0, StatusCode::UNAUTHORIZED, "JSON gets 401");
    assert_eq!(send(&r, jar, Method::GET, "/private", false).await.1, "/login", "require layer");
    assert_eq!(send(&r, jar, Method::GET, "/guest", false).await.1, "welcome");

    send(&r, jar, Method::POST, "/login/1", false).await;
    let first = jar.clone();
    assert_eq!(send(&r, jar, Method::GET, "/me", false).await.1, "ada");
    assert_eq!(send(&r, jar, Method::GET, "/private", false).await.1, "secret");
    assert_eq!(send(&r, jar, Method::GET, "/guest", false).await, (StatusCode::SEE_OTHER, "/".into()), "guest redirects home");

    send(&r, jar, Method::POST, "/logout", false).await;
    assert_eq!(send(&r, jar, Method::GET, "/me", true).await.0, StatusCode::UNAUTHORIZED, "logged out");
    let mut old = first;
    assert_eq!(send(&r, &mut old, Method::GET, "/me", true).await.0, StatusCode::UNAUTHORIZED, "old session id is dead");

    send(&r, jar, Method::POST, "/login/2", false).await;
    assert_eq!(send(&r, jar, Method::GET, "/me", true).await.0, StatusCode::UNAUTHORIZED, "deleted user is logged out");
}
