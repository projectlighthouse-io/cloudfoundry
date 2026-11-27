#![cfg(any(feature = "gitlab", feature = "reddit", feature = "twitter", feature = "oidc", feature = "auth0"))]

//! Each provider against a fake of its API: the authorize URL it builds, how it
//! authenticates at the token endpoint, and the `SocialUser` it produces.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use route::{
    Cx, Json, Query, Router, Routes, empty,
    http::{Request, header},
};
use securecookie::{Codec, Keys};
use serde_json::json;
use session::{MemoryStore, Session};
use socialite::{Callback, Provider, SocialUser};

/// What each token endpoint received: path -> (Authorization header, form).
type Seen = Arc<Mutex<HashMap<String, (Option<String>, HashMap<String, String>)>>>;

async fn token(cx: Cx<Seen>, req: Request<route::Body>) -> Json<serde_json::Value> {
    let path = req.uri().path().to_owned();
    let auth = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let bytes = http_body_util::BodyExt::collect(req.into_body()).await.unwrap().to_bytes();
    let form: HashMap<String, String> = serde_urlencoded::from_bytes(&bytes).unwrap();
    cx.lock().unwrap().insert(path, (auth, form));
    Json(json!({ "access_token": "tok", "token_type": "bearer" }))
}

async fn fake(seen: Seen) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut r = Router::new();
    for path in ["/gitlab/oauth/token", "/reddit/api/v1/access_token", "/x-api/2/oauth2/token", "/oidc/token", "/auth0/oauth/token"] {
        r.post(path, token);
    }
    let claims = |sub: &str, email: &str| json!({ "sub": sub, "email": email, "email_verified": true, "name": "Ada", "picture": "https://img/a.png", "groups": ["admins"] });
    let (g, o, a) = (claims("11", "ada@gitlab.test"), claims("22", "ada@okta.test"), claims("auth0|33", "ada@auth0.test"));
    r.get("/gitlab/oauth/userinfo", move |_: ()| { let g = g.clone(); async move { Json(g) } });
    r.get("/oidc/userinfo", move |_: ()| { let o = o.clone(); async move { Json(o) } });
    r.get("/auth0/userinfo", move |_: ()| { let a = a.clone(); async move { Json(a) } });
    r.get("/reddit-api/api/v1/me", |_: ()| async { Json(json!({ "id": "t2abc", "name": "ada", "icon_img": "https://img/r.png?a=1&amp;b=2" })) });
    r.get("/x-api/2/users/me", |_: ()| async { Json(json!({ "data": { "id": "44", "name": "Ada", "username": "ada", "profile_image_url": "https://img/x.png" } })) });
    let b = base.clone();
    r.get("/oidc/.well-known/openid-configuration", move |_: ()| {
        let b = b.clone();
        async move { Json(json!({ "authorization_endpoint": format!("{b}/oidc/authorize"), "token_endpoint": format!("{b}/oidc/token"), "userinfo_endpoint": format!("{b}/oidc/userinfo") })) }
    });
    tokio::spawn(route::serve(listener, r.build(seen).unwrap()));
    base
}

/// Run the redirect and callback for `p`; return the authorize URL and the user.
async fn login<P: Provider + 'static>(p: P) -> (String, SocialUser) {
    async fn go<P: Provider>(cx: Cx<P>, s: Session) -> String {
        cx.authorize_url(&s)
    }
    async fn back<P: Provider>(cx: Cx<P>, (s, Query(cb)): (Session, Query<Callback>)) -> String {
        let u = cx.social_user(&s, cb).await.unwrap();
        serde_json::to_string(&json!([u.provider, u.id, u.name, u.email, u.email_verified, u.avatar, u.token])).unwrap()
    }
    let mut r = Router::new();
    r.get("/go", go::<P>);
    r.get("/cb", back::<P>);
    r.layer(session::layer(MemoryStore::new(), Codec::new(Keys::parse(&Keys::generate()).unwrap())));
    let r: Routes = r.build(p).unwrap();

    let mut cookie = String::new();
    let url = call(&r, "/go", &mut cookie).await;
    let query: HashMap<String, String> = serde_urlencoded::from_str(url.split_once('?').unwrap().1).unwrap();
    let out = call(&r, &format!("/cb?code=c&state={}", query["state"]), &mut cookie).await;
    let v: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    let s = |i: usize| v[i].as_str().map(str::to_owned);
    let user = SocialUser {
        provider: Box::leak(s(0).unwrap().into_boxed_str()),
        id: s(1).unwrap(),
        name: s(2),
        email: s(3),
        email_verified: v[4].as_bool(),
        avatar: s(5),
        token: s(6).unwrap(),
        refresh_token: None,
    };
    (url, user)
}

async fn call(r: &Routes, uri: &str, cookie: &mut String) -> String {
    let mut req = Request::get(uri);
    if !cookie.is_empty() {
        req = req.header(header::COOKIE, cookie.as_str());
    }
    let res = r.handle(req.body(empty()).unwrap()).await;
    if let Some(c) = res.headers().get(header::SET_COOKIE) {
        *cookie = c.to_str().unwrap().split(';').next().unwrap().to_owned();
    }
    String::from_utf8(http_body_util::BodyExt::collect(res.into_body()).await.unwrap().to_bytes().to_vec()).unwrap()
}

fn basic(id: &str, secret: &str) -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    format!("Basic {}", STANDARD.encode(format!("{id}:{secret}")))
}

#[cfg(feature = "gitlab")]
#[tokio::test(flavor = "multi_thread")]
async fn gitlab_self_hosted() {
    let seen = Seen::default();
    let base = fake(seen.clone()).await;
    let (url, u) = login(socialite::GitLab::new("id", "secret", "http://app/cb").base_url(format!("{base}/gitlab"))).await;
    assert!(url.starts_with(&format!("{base}/gitlab/oauth/authorize?")) && url.contains("code_challenge="));
    assert_eq!((u.provider, u.id.as_str(), u.email.as_deref(), u.email_verified), ("gitlab", "11", Some("ada@gitlab.test"), Some(true)));
    let (auth, form) = seen.lock().unwrap()["/gitlab/oauth/token"].clone();
    assert_eq!((auth, form["client_secret"].as_str()), (None, "secret"), "secret in the form");
    assert!(form.contains_key("code_verifier"));
}

#[cfg(feature = "reddit")]
#[tokio::test(flavor = "multi_thread")]
async fn reddit_basic_auth_no_pkce_no_email() {
    let seen = Seen::default();
    let base = fake(seen.clone()).await;
    let (url, u) = login(socialite::Reddit::new("id", "secret", "http://app/cb").base_urls(format!("{base}/reddit"), format!("{base}/reddit-api"))).await;
    assert!(url.contains("duration=temporary") && url.contains("scope=identity") && !url.contains("code_challenge"));
    assert_eq!((u.provider, u.id.as_str(), u.name.as_deref(), u.email), ("reddit", "t2abc", Some("ada"), None));
    assert_eq!(u.avatar.as_deref(), Some("https://img/r.png?a=1&b=2"), "&amp; unescaped");
    let (auth, form) = seen.lock().unwrap()["/reddit/api/v1/access_token"].clone();
    assert_eq!(auth.as_deref(), Some(basic("id", "secret").as_str()));
    assert!(!form.contains_key("client_secret") && !form.contains_key("code_verifier"));
}

#[cfg(feature = "twitter")]
#[tokio::test(flavor = "multi_thread")]
async fn twitter_basic_auth_with_pkce() {
    let seen = Seen::default();
    let base = fake(seen.clone()).await;
    let (url, u) = login(socialite::Twitter::new("id", "secret", "http://app/cb").base_urls(format!("{base}/x"), format!("{base}/x-api"))).await;
    assert!(url.starts_with(&format!("{base}/x/i/oauth2/authorize?")) && url.contains("scope=users.read+tweet.read") && url.contains("code_challenge="));
    assert_eq!((u.provider, u.id.as_str(), u.name.as_deref(), u.email, u.avatar.as_deref()), ("twitter", "44", Some("Ada"), None, Some("https://img/x.png")));
    let (auth, form) = seen.lock().unwrap()["/x-api/2/oauth2/token"].clone();
    assert_eq!(auth.as_deref(), Some(basic("id", "secret").as_str()));
    assert!(form.contains_key("code_verifier"));
}

#[cfg(feature = "oidc")]
#[tokio::test(flavor = "multi_thread")]
async fn oidc_discovery_and_own_name() {
    let base = fake(Seen::default()).await;
    let okta = socialite::Oidc::discover("okta", &format!("{base}/oidc/"), "id", "secret", "http://app/cb").await.unwrap();
    let (url, u) = login(okta).await;
    assert!(url.starts_with(&format!("{base}/oidc/authorize?")) && url.contains("scope=openid+email+profile"));
    assert_eq!((u.provider, u.id.as_str(), u.email.as_deref(), u.email_verified), ("okta", "22", Some("ada@okta.test"), Some(true)));
}

#[cfg(feature = "oidc")]
#[tokio::test(flavor = "multi_thread")]
async fn oidc_keeps_custom_claims() {
    let base = fake(Seen::default()).await;
    let p = socialite::Oidc::discover("okta", &format!("{base}/oidc"), "id", "secret", "http://app/cb").await.unwrap();
    let user = p.fetch_user(socialite::Token { access_token: "tok".into(), refresh_token: None, expires_in: None }).await.unwrap();
    assert_eq!(user.claims["groups"][0], "admins");
}

#[cfg(feature = "auth0")]
#[tokio::test(flavor = "multi_thread")]
async fn auth0_tenant() {
    let base = fake(Seen::default()).await;
    let (url, u) = login(socialite::Auth0::with_base(&format!("{base}/auth0"), "id", "secret", "http://app/cb")).await;
    assert!(url.starts_with(&format!("{base}/auth0/authorize?")));
    assert_eq!((u.provider, u.id.as_str(), u.email.as_deref()), ("auth0", "auth0|33", Some("ada@auth0.test")));
}
