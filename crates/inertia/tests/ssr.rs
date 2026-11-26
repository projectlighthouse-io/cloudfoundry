#![cfg(feature = "ssr")]

use http_body_util::BodyExt;
use route::{
    Json, Router, Routes, empty,
    http::{Request, StatusCode},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct PageIn {
    component: String,
}

#[derive(Serialize)]
struct Rendered {
    head: Vec<String>,
    body: String,
}

/// A stand-in for the Node SSR server.
async fn fake_ssr(fail: bool) -> String {
    let mut r = Router::new();
    r.post("/render", move |Json(p): Json<PageIn>| async move {
        if fail {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        Ok(Json(Rendered {
            head: vec!["<title>SSR</title>".into()],
            body: format!(r#"<div id="app" data-server-rendered="true">{}</div>"#, p.component),
        }))
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(route::serve(listener, r.build(()).unwrap()));
    url
}

fn app(ssr: &str) -> Routes {
    let mut r = Router::new();
    r.get("/", |_: ()| async { inertia::render("home", ()) });
    r.layer(inertia::layer().ssr(ssr).head(|| "<x-assets>".into()));
    r.build(()).unwrap()
}

async fn first_visit(r: &Routes) -> String {
    let res = r.handle(Request::get("/").body(empty()).unwrap()).await;
    String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
}

#[tokio::test]
async fn renders_on_the_ssr_server() {
    let html = first_visit(&app(&fake_ssr(false).await)).await;
    assert!(html.contains("<x-assets><title>SSR</title>"), "{html}");
    assert!(html.contains(r#"<div id="app" data-server-rendered="true">home</div>"#), "{html}");
    assert!(!html.contains("data-page"), "SSR body replaces the script and root div");
}

#[tokio::test]
async fn falls_back_to_client_rendering() {
    for url in [fake_ssr(true).await, "http://127.0.0.1:9".into()] {
        let html = first_visit(&app(&url)).await;
        assert!(html.contains(r#"<script data-page="app" type="application/json">"#), "{url}: {html}");
    }
}
