#![cfg(feature = "route")]

use std::time::Duration;

use http_body_util::BodyExt;
use route::{
    Router,
    http::{Request, StatusCode, header, request::Parts},
};
use throttle::{FixedWindow, Limiter};

fn by_header(parts: &Parts) -> Option<String> {
    parts.headers.get("x-client").and_then(|v| v.to_str().ok()).map(str::to_owned)
}

async fn ok(_: ()) -> &'static str {
    "ok"
}

#[tokio::test]
async fn limits_per_key_with_retry_after() {
    let mut r = Router::new();
    r.layer(throttle::layer("api", Limiter::memory(FixedWindow::new(2, Duration::from_secs(60))), by_header));
    r.get("/", ok);
    let routes = r.build(()).unwrap();
    let call = |client: Option<&'static str>| {
        let routes = routes.clone();
        async move {
            let mut req = Request::get("/");
            if let Some(c) = client {
                req = req.header("x-client", c);
            }
            routes.handle(req.body(route::empty()).unwrap()).await
        }
    };

    assert_eq!(call(Some("a")).await.status(), StatusCode::OK);
    assert_eq!(call(Some("a")).await.status(), StatusCode::OK);
    let res = call(Some("a")).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry: u64 = res.headers()[header::RETRY_AFTER].to_str().unwrap().parse().unwrap();
    assert!((59..=60).contains(&retry), "{retry}");

    assert_eq!(call(Some("b")).await.status(), StatusCode::OK, "other key unaffected");
    let res = call(None).await;
    assert_eq!(res.status(), StatusCode::OK, "no key: not counted");
    assert_eq!(res.into_body().collect().await.unwrap().to_bytes(), "ok");
}

#[tokio::test]
async fn by_ip_uses_the_client_address() {
    let mut r = Router::new();
    r.layer(throttle::layer("web", Limiter::memory(FixedWindow::new(1, Duration::from_secs(60))), throttle::by_ip));
    r.get("/", ok);
    let routes = r.build(()).unwrap();
    let call = |ip: [u8; 4]| {
        let mut req = Request::get("/").body(route::empty()).unwrap();
        req.extensions_mut().insert(route::ClientAddr((ip, 4000).into()));
        routes.handle(req)
    };
    assert_eq!(call([10, 0, 0, 1]).await.status(), StatusCode::OK);
    assert_eq!(call([10, 0, 0, 1]).await.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(call([10, 0, 0, 2]).await.status(), StatusCode::OK);
}
