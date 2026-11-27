#![cfg(feature = "github")]

use route::{
    Cx, Json, Query, Router, Routes, empty,
    http::{Request, header},
};
use securecookie::{Codec, Keys};
use session::{MemoryStore, Session};
use socialite::{Callback, GitHub, Provider};

/// Fake github.com and api.github.com on one local port.
async fn fake_github() -> String {
    let mut r = Router::new();
    // The token is the code, so a test picks which user:email grant it gets.
    r.post("/login/oauth/access_token", |route::Form(f): route::Form<std::collections::HashMap<String, String>>| async move {
        Json(serde_json::json!({ "access_token": f["code"] }))
    });
    r.get("/user", |_: ()| async { Json(UserJson { id: 7, login: "octo" }) });
    // Only the token "tok" was granted `user:email`.
    r.get("/user/emails", |req: Request<route::Body>| async move {
        let granted = req.headers().get(header::AUTHORIZATION).is_some_and(|v| v == "Bearer tok");
        if granted {
            Ok(Json(serde_json::json!([
                { "email": "old@octo.dev", "primary": false, "verified": true },
                { "email": "octo@octo.dev", "primary": true, "verified": true }
            ])))
        } else {
            Err(route::http::StatusCode::NOT_FOUND)
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(route::serve(listener, r.build(()).unwrap()));
    base
}

#[derive(serde::Serialize)]
struct UserJson {
    id: u64,
    login: &'static str,
}

async fn go(cx: Cx<GitHub>, session: Session) -> String {
    cx.authorize_url(&session)
}

async fn back(cx: Cx<GitHub>, (session, Query(cb)): (Session, Query<Callback>)) -> String {
    match cx.user(&session, cb).await {
        Ok(u) => format!("{} {} {} {:?}", u.login, u.token, u.email.unwrap_or_default(), u.email_verified),
        Err(e) => e.to_string(),
    }
}

fn app(base: &str) -> Routes {
    let gh = GitHub::new("id", "secret", "http://app/cb").base_urls(base, base);
    let mut r = Router::new();
    r.get("/go", go);
    r.get("/cb", back);
    r.layer(session::layer(MemoryStore::new(), Codec::new(Keys::parse(&Keys::generate()).unwrap())));
    r.build(gh).unwrap()
}

async fn get(r: &Routes, uri: &str, cookie: &mut String) -> String {
    let mut req = Request::get(uri);
    if !cookie.is_empty() {
        req = req.header(header::COOKIE, cookie.as_str());
    }
    let res = r.handle(req.body(empty()).unwrap()).await;
    if let Some(c) = res.headers().get(header::SET_COOKIE) {
        *cookie = c.to_str().unwrap().split(';').next().unwrap().to_owned();
    }
    let bytes = http_body_util::BodyExt::collect(res.into_body()).await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn github_login_checks_state_once() {
    let base = fake_github().await;
    let r = app(&base);
    let cookie = &mut String::new();

    let url = get(&r, "/go", cookie).await;
    assert!(url.starts_with(&format!("{base}/login/oauth/authorize?client_id=id")));
    let state = url.split("state=").nth(1).unwrap().to_owned();

    assert_eq!(get(&r, &format!("/cb?code=tok&state={state}"), cookie).await, "octo tok octo@octo.dev Some(true)", "verified primary email from /user/emails");
    assert_eq!(get(&r, &format!("/cb?code=tok&state={state}"), cookie).await, "oauth state mismatch", "state is single use");

    let state = get(&r, "/go", cookie).await.split("state=").nth(1).unwrap().to_owned();
    assert_eq!(get(&r, &format!("/cb?code=noscope&state={state}"), cookie).await, "octo noscope  None", "without user:email: no email, verification unknown");

    get(&r, "/go", cookie).await;
    assert_eq!(get(&r, "/cb?code=c&state=forged", cookie).await, "oauth state mismatch");
}
