use http_body_util::BodyExt;
use inertia::{Defer, Optional, Page, Visit};
use route::{
    Router, Routes, empty,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use serde::Serialize;

#[derive(Serialize)]
struct Show {
    title: &'static str,
    logs: Optional<Vec<u32>>,
    artifacts: Defer<Vec<&'static str>>,
    side: Defer<u32>,
}

async fn show(visit: Visit) -> Page<Show> {
    inertia::render(
        "runs/show",
        Show {
            title: "</script><b>",
            logs: visit.optional("logs", async { vec![1, 2] }).await,
            artifacts: visit.defer("artifacts", async { vec!["a.tar"] }).await,
            side: if visit.wants("side") { Defer::loaded(9).group("side") } else { Defer::new().group("side") },
        },
    )
}

struct Response302;
impl route::IntoResponse for Response302 {
    fn into_response(self) -> route::http::Response<route::Body> {
        let mut r = StatusCode::FOUND.into_response();
        r.headers_mut().insert(header::LOCATION, "/runs/1".parse().unwrap());
        r
    }
}
fn app() -> Routes {
    let mut r = Router::new();
    r.get("/runs/1", show);
    r.put("/runs/1", |_: ()| async { Response302 });
    r.get("/plain", |_: ()| async { "plain" });
    r.layer(inertia::layer().version("v1").head(|| "<x-head>".into()));
    r.build(()).unwrap()
}

async fn send(r: &Routes, method: Method, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, HeaderMap, String) {
    let mut req = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = r.handle(req.body(empty()).unwrap()).await;
    let (parts, b) = res.into_parts();
    (parts.status, parts.headers, String::from_utf8(b.collect().await.unwrap().to_bytes().to_vec()).unwrap())
}

const INERTIA: (&str, &str) = ("x-inertia", "true");
const V1: (&str, &str) = ("x-inertia-version", "v1");

#[tokio::test]
async fn first_visit_is_html_with_escaped_page() {
    let (status, h, html) = send(&app(), Method::GET, "/runs/1", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(h[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"));
    assert_eq!(h[header::VARY], "X-Inertia");
    assert!(html.contains("<x-head>"));
    assert!(html.contains(r#"<script data-page="app" type="application/json">{"component":"runs/show""#));
    let escaped = "</script><b>".replace('<', &format!("{}u003c", '\\'));
    assert!(html.contains(&escaped) && !html.contains("</script><b>"), "page JSON cannot close the script: {html}");
    assert!(html.contains(r#"<div id="app"></div>"#));
}

#[tokio::test]
async fn inertia_visit_announces_deferred_and_hides_optional() {
    let (status, h, json) = send(&app(), Method::GET, "/runs/1", &[INERTIA, V1]).await;
    assert_eq!((status, &h["x-inertia"]), (StatusCode::OK, &"true".parse::<route::http::HeaderValue>().unwrap()));
    assert_eq!(
        json,
        r#"{"component":"runs/show","props":{"errors":{},"title":"</script><b>"},"url":"/runs/1","version":"v1","deferredProps":{"default":["artifacts"],"side":["side"]}}"#
    );
}

#[tokio::test]
async fn partial_reloads() {
    let partial = |data: &'static str| [INERTIA, V1, ("x-inertia-partial-component", "runs/show"), ("x-inertia-partial-data", data)];
    let (_, _, json) = send(&app(), Method::GET, "/runs/1", &partial("artifacts,logs")).await;
    assert_eq!(json, r#"{"component":"runs/show","props":{"artifacts":["a.tar"],"errors":{},"logs":[1,2]},"url":"/runs/1","version":"v1"}"#);

    let (_, _, json) = send(&app(), Method::GET, "/runs/1", &partial("side")).await;
    assert!(json.contains(r#""props":{"errors":{},"side":9}"#), "{json}");

    // Except alone selects every other prop, so optional and deferred ones load too.
    let except = [INERTIA, V1, ("x-inertia-partial-component", "runs/show"), ("x-inertia-partial-except", "title")];
    assert!(send(&app(), Method::GET, "/runs/1", &except).await.2.contains(r#""props":{"artifacts":["a.tar"],"errors":{},"logs":[1,2],"side":9}"#));

    // A partial reload naming another component is a full visit.
    let other = [INERTIA, V1, ("x-inertia-partial-component", "other"), ("x-inertia-partial-data", "logs")];
    assert!(send(&app(), Method::GET, "/runs/1", &other).await.2.contains("deferredProps"));
}

#[tokio::test]
async fn version_mismatch_and_redirects() {
    let (status, h, _) = send(&app(), Method::GET, "/runs/1?x=1", &[INERTIA, ("x-inertia-version", "old")]).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!((&h["x-inertia-location"], &h["x-inertia-version"]), (&"/runs/1?x=1".parse::<route::http::HeaderValue>().unwrap(), &"v1".parse::<route::http::HeaderValue>().unwrap()));
    assert!(!h.contains_key("x-inertia"), "409 carries no X-Inertia");

    assert_eq!(send(&app(), Method::PUT, "/runs/1", &[INERTIA, V1]).await.0, StatusCode::SEE_OTHER, "302 after PUT");
    assert_eq!(send(&app(), Method::GET, "/plain", &[INERTIA, V1]).await.2, "plain", "non-page passes through");
}
