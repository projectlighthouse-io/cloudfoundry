#![cfg(all(feature = "route", feature = "google"))]

use route::{
    Json, Router, Routes, empty,
    http::{Request, StatusCode, header},
};
use securecookie::{Codec, Keys};
use session::MemoryStore;
use socialite::{Google, SocialCallback};

async fn fake_google() -> String {
    let mut r = Router::new();
    r.post("/token", |_: ()| async { Json(serde_json::json!({ "access_token": "tok" })) });
    r.get("/userinfo", |_: ()| async { Json(serde_json::json!({ "sub": "9", "email": "ada@gmail.com", "email_verified": true, "name": "Ada" })) });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(route::serve(listener, r.build(()).unwrap()));
    base
}

async fn callback(social: SocialCallback) -> String {
    match social.0 {
        Ok(u) => format!("{} {} {}", u.provider, u.id, u.email.unwrap_or_default()),
        Err(e) => format!("error: {e}"),
    }
}

fn app(base: &str, with_session: bool) -> Routes {
    let google = Google::new("id", "secret", "http://app/auth/google/callback").base_urls(format!("{base}/auth"), format!("{base}/token"), format!("{base}/userinfo"));
    let mut r = Router::new();
    socialite::routes(&mut r, google, callback);
    r.get("/names", |u: route::http::Request<route::Body>| async move {
        let urls = u.extensions().get::<route::UrlFor>().cloned().unwrap();
        format!("{} {}", urls.url("social.google", &[]).unwrap(), urls.url("social.google.callback", &[]).unwrap())
    });
    if with_session {
        r.layer(session::layer(MemoryStore::new(), Codec::new(Keys::parse(&Keys::generate()).unwrap())));
    }
    r.build(()).unwrap()
}

async fn get(r: &Routes, uri: &str, cookie: &mut String) -> (StatusCode, String) {
    let mut req = Request::get(uri);
    if !cookie.is_empty() {
        req = req.header(header::COOKIE, cookie.as_str());
    }
    let res = r.handle(req.body(empty()).unwrap()).await;
    if let Some(c) = res.headers().get(header::SET_COOKIE) {
        *cookie = c.to_str().unwrap().split(';').next().unwrap().to_owned();
    }
    let status = res.status();
    let loc = res.headers().get(header::LOCATION).map(|v| v.to_str().unwrap().to_owned());
    let body = String::from_utf8(http_body_util::BodyExt::collect(res.into_body()).await.unwrap().to_bytes().to_vec()).unwrap();
    (status, loc.unwrap_or(body))
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_and_callback_routes() {
    let base = fake_google().await;
    let r = app(&base, true);
    let cookie = &mut String::new();

    assert_eq!(get(&r, "/names", cookie).await.1, "/auth/google /auth/google/callback", "named routes");

    let (status, to) = get(&r, "/auth/google", cookie).await;
    assert!(status.is_redirection(), "{status}");
    assert!(to.starts_with(&format!("{base}/auth?")), "{to}");
    let state = to.split("state=").nth(1).unwrap().split('&').next().unwrap().to_owned();

    assert_eq!(get(&r, &format!("/auth/google/callback?code=c&state={state}"), cookie).await.1, "google 9 ada@gmail.com");
    assert_eq!(get(&r, &format!("/auth/google/callback?code=c&state={state}"), cookie).await.1, "error: oauth state mismatch", "the app decides what an error means");
    get(&r, "/auth/google", cookie).await;
    assert_eq!(get(&r, "/auth/google/callback?error=access_denied&state=x", cookie).await.1, "error: oauth state mismatch");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_session_layer_the_callback_says_so() {
    let base = fake_google().await;
    let r = app(&base, false);
    let (status, body) = get(&r, "/auth/google/callback?code=c&state=s", &mut String::new()).await;
    assert_eq!((status, body.as_str()), (StatusCode::INTERNAL_SERVER_ERROR, "socialite needs the session layer"));
}
