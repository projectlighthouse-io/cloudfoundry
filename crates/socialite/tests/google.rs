#![cfg(feature = "google")]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use route::{
    Cx, Form, Json, Router, Routes, empty,
    http::{Request, header},
};
use securecookie::{Codec, Keys};
use session::{MemoryStore, Session};
use sha2::{Digest, Sha256};
use socialite::{Callback, Google, Provider, SocialUser};

/// The token request the fake Google received last.
type Seen = Arc<Mutex<HashMap<String, String>>>;

/// Fake Google: the access token is the code it was given, and the userinfo
/// answer depends on it ("acme" is a Workspace user, anything else is not).
async fn fake_google(seen: Seen) -> String {
    let mut r = Router::new();
    r.post("/token", |cx: Cx<Seen>, Form(f): Form<HashMap<String, String>>| async move {
        let code = f.get("code").cloned().unwrap_or_default();
        *cx.lock().unwrap() = f;
        Json(serde_json::json!({ "access_token": code, "refresh_token": "refresh", "expires_in": 3599 }))
    });
    r.get("/userinfo", |req: Request<route::Body>| async move {
        let token = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or_default().trim_start_matches("Bearer ").to_owned();
        Json(match token.as_str() {
            "acme" => serde_json::json!({ "sub": "1", "email": "ada@acme.com", "email_verified": true, "name": "Ada", "hd": "acme.com" }),
            _ => serde_json::json!({ "sub": "2", "email": "bob@gmail.com", "email_verified": false, "name": "Bob" }),
        })
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(route::serve(listener, r.build(seen).unwrap()));
    base
}

async fn go(cx: Cx<Google>, session: Session) -> String {
    cx.authorize_url(&session)
}

async fn back(cx: Cx<Google>, (session, route::Query(cb)): (Session, route::Query<Callback>)) -> String {
    match cx.user(&session, cb).await {
        Ok(u) => {
            let s: SocialUser = u.into();
            format!("{} {} {:?} {:?} {}", s.provider, s.id, s.email, s.email_verified, s.refresh_token.unwrap_or_default())
        }
        Err(e) => e.to_string(),
    }
}

fn app(google: Google) -> Routes {
    let mut r = Router::new();
    r.get("/go", go);
    r.get("/cb", back);
    r.layer(session::layer(MemoryStore::new(), Codec::new(Keys::parse(&Keys::generate()).unwrap())));
    r.build(google).unwrap()
}

fn google(base: &str) -> Google {
    Google::new("id", "secret", "http://app/cb").base_urls(format!("{base}/auth"), format!("{base}/token"), format!("{base}/userinfo"))
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

fn param(url: &str, name: &str) -> String {
    let query = url.split_once('?').unwrap().1;
    let pairs: HashMap<String, String> = serde_urlencoded::from_str(query).unwrap();
    pairs[name].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn google_login_with_pkce() {
    let seen = Seen::default();
    let base = fake_google(seen.clone()).await;
    let r = app(google(&base).offline().prompt("select_account"));
    let cookie = &mut String::new();

    let url = get(&r, "/go", cookie).await;
    assert!(url.starts_with(&format!("{base}/auth?")));
    assert_eq!(param(&url, "scope"), "openid email profile");
    assert_eq!(param(&url, "response_type"), "code");
    assert_eq!(param(&url, "access_type"), "offline");
    assert_eq!(param(&url, "prompt"), "select_account");
    assert_eq!(param(&url, "code_challenge_method"), "S256");
    let (state, challenge) = (param(&url, "state"), param(&url, "code_challenge"));

    let out = get(&r, &format!("/cb?code=plain&state={state}"), cookie).await;
    assert_eq!(out, r#"google 2 Some("bob@gmail.com") Some(false) refresh"#, "unverified email is reported, not hidden");

    let sent = seen.lock().unwrap().clone();
    assert_eq!(sent["grant_type"], "authorization_code");
    assert_eq!(sent["redirect_uri"], "http://app/cb");
    assert_eq!(URL_SAFE_NO_PAD.encode(Sha256::digest(sent["code_verifier"].as_bytes())), challenge, "verifier matches the challenge");

    assert_eq!(get(&r, &format!("/cb?code=plain&state={state}"), cookie).await, "oauth state mismatch", "state is single use");
    get(&r, "/go", cookie).await;
    assert_eq!(get(&r, "/cb?code=plain&state=forged", cookie).await, "oauth state mismatch");
}

#[tokio::test(flavor = "multi_thread")]
async fn hosted_domain_is_checked_not_just_hinted() {
    let base = fake_google(Seen::default()).await;
    let r = app(google(&base).hosted_domain("acme.com"));
    let cookie = &mut String::new();

    let url = get(&r, "/go", cookie).await;
    assert_eq!(param(&url, "hd"), "acme.com");
    let state = param(&url, "state");
    assert!(get(&r, &format!("/cb?code=acme&state={state}"), cookie).await.starts_with("google 1"));

    let state = param(&get(&r, "/go", cookie).await, "state");
    assert_eq!(get(&r, &format!("/cb?code=plain&state={state}"), cookie).await, "oauth denied: account is not in the acme.com domain");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_denied_consent_is_an_error() {
    let base = fake_google(Seen::default()).await;
    let r = app(google(&base));
    let cookie = &mut String::new();
    let state = param(&get(&r, "/go", cookie).await, "state");
    assert_eq!(get(&r, &format!("/cb?error=access_denied&state={state}"), cookie).await, "oauth denied: access_denied");
}
