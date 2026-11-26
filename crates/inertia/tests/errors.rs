#![cfg(feature = "validate")]

use http_body_util::BodyExt;
use route::{
    Json, Router, Routes, body,
    http::{Method, Request, StatusCode, header},
};
use securecookie::{Codec, Keys};
use serde::Deserialize;
use session::MemoryStore;
use validate::{Errors, Valid, Validate, required};

#[derive(Deserialize)]
struct New {
    name: String,
}

impl Validate for New {
    fn validate(&self, e: &mut Errors) {
        e.check("name", required(&self.name)).check("name", validate::min_len(&self.name, 3));
    }
}

fn app() -> Routes {
    let mut r = Router::new();
    r.get("/form", |_: ()| async { inertia::render("form", ()) });
    r.post("/save", |_: Valid<Json<New>>| async { "saved" });
    r.layer(session::layer(MemoryStore::new(), Codec::new(Keys::parse(&Keys::generate()).unwrap())))
        .layer(inertia::layer());
    r.build(()).unwrap()
}

#[tokio::test]
async fn validation_failure_redirects_back_with_first_errors() {
    let r = app();
    let post = Request::builder()
        .method(Method::POST)
        .uri("/save")
        .header("x-inertia", "true")
        .header(header::REFERER, "/form")
        .header(header::CONTENT_TYPE, "application/json")
        .body(body(r#"{"name":""}"#))
        .unwrap();
    let res = r.handle(post).await;
    assert_eq!((res.status(), &res.headers()[header::LOCATION]), (StatusCode::SEE_OTHER, &"/form".parse::<route::http::HeaderValue>().unwrap()));
    let cookie = res.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_owned();

    let get = Request::get("/form").header("x-inertia", "true").header(header::COOKIE, &cookie).body(route::empty()).unwrap();
    let page = r.handle(get).await.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&page).contains(r#""errors":{"name":"This field is required."}"#), "first message per field");

    // With an error bag, the errors arrive under its name.
    let bagged = Request::builder()
        .method(Method::POST)
        .uri("/save")
        .header("x-inertia", "true")
        .header("x-inertia-error-bag", "createCompany")
        .header(header::COOKIE, &cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body(r#"{"name":"ab"}"#))
        .unwrap();
    let res = r.handle(bagged).await;
    let cookie = res.headers().get(header::SET_COOKIE).map(|c| c.to_str().unwrap().split(';').next().unwrap().to_owned()).unwrap_or(cookie);
    let get = Request::get("/form").header("x-inertia", "true").header(header::COOKIE, &cookie).body(route::empty()).unwrap();
    let page = r.handle(get).await.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&page).contains(r#""errors":{"createCompany":{"name":"Must be at least 3 characters."}}"#), "{}", String::from_utf8_lossy(&page));

    // Without X-Inertia the plain 422 stays.
    let plain = Request::post("/save").header(header::CONTENT_TYPE, "application/json").body(body(r#"{"name":""}"#)).unwrap();
    assert_eq!(r.handle(plain).await.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
