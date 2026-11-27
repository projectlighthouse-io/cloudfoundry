#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
use http_body_util::BodyExt;
use inertia::{Defer, Merge, Once, Optional, Page, Visit};
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

#[derive(Serialize)]
struct Feed {
    items: Merge<Vec<u32>>,
    plans: Once<Vec<&'static str>>,
}

async fn feed(visit: Visit) -> Page<Feed> {
    inertia::render(
        "feed",
        Feed {
            items: Merge::new(vec![1, 2]).match_on("id"),
            plans: visit.once("plans", async { vec!["pro"] }).await,
        },
    )
}

fn feed_app(pages: Option<&str>) -> Routes {
    let mut r = Router::new();
    r.get("/feed", feed);
    let mut l = inertia::layer().version("v1");
    if let Some(dir) = pages {
        l = l.pages_dir(dir);
    }
    r.layer(l);
    r.build(()).unwrap()
}

#[tokio::test]
async fn merge_and_once_props() {
    let app = feed_app(None);
    let full = send(&app, Method::GET, "/feed", &[INERTIA, V1]).await.2;
    assert_eq!(
        full,
        r#"{"component":"feed","props":{"errors":{},"items":[1,2],"plans":["pro"]},"url":"/feed","version":"v1","mergeProps":["items"],"matchPropsOn":["items.id"],"onceProps":{"plans":{"prop":"plans","expiresAt":null}}}"#
    );

    // The client still has `plans`: skipped, but still announced.
    let has = [INERTIA, V1, ("x-inertia-except-once-props", "plans")];
    let json = send(&app, Method::GET, "/feed", &has).await.2;
    assert!(json.contains(r#""props":{"errors":{},"items":[1,2]}"#) && json.contains(r#""onceProps":{"plans""#), "{json}");

    // A partial reload asking for it always gets it, despite the except-once header.
    let ask = [INERTIA, V1, ("x-inertia-except-once-props", "plans"), ("x-inertia-partial-component", "feed"), ("x-inertia-partial-data", "plans")];
    assert!(send(&app, Method::GET, "/feed", &ask).await.2.contains(r#""props":{"errors":{},"plans":["pro"]}"#));

    // A reset prop comes back without its merge label.
    let reset = [INERTIA, V1, ("x-inertia-partial-component", "feed"), ("x-inertia-partial-data", "items"), ("x-inertia-reset", "items")];
    let json = send(&app, Method::GET, "/feed", &reset).await.2;
    assert!(json.contains(r#""items":[1,2]"#) && !json.contains("mergeProps") && !json.contains("matchPropsOn"), "{json}");
}

#[tokio::test]
async fn missing_page_component_is_an_error_in_dev() {
    let dir = std::env::temp_dir().join(format!("inertia-pages-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let app = feed_app(Some(dir.to_str().unwrap()));
    assert_eq!(send(&app, Method::GET, "/feed", &[INERTIA, V1]).await.0, StatusCode::INTERNAL_SERVER_ERROR);
    std::fs::write(dir.join("feed.vue"), "").unwrap();
    assert_eq!(send(&app, Method::GET, "/feed", &[INERTIA, V1]).await.0, StatusCode::OK);
}

#[derive(Serialize)]
struct Log {
    lines: inertia::Scroll<Vec<u64>>,
}

async fn log(route::Query(q): route::Query<std::collections::HashMap<String, u64>>) -> Page<Log> {
    let page = q.get("page").copied().unwrap_or(2);
    let lines = vec![page * 10, page * 10 + 1];
    inertia::render(
        "log",
        Log { lines: inertia::Scroll::new(lines, page).previous(page.checked_sub(1).filter(|p| *p > 0)).next((page < 3).then_some(page + 1)).match_on("id") },
    )
}

fn log_app() -> Routes {
    let mut r = Router::new();
    r.get("/log", log);
    r.get("/secret", |_: ()| async { inertia::render("secret", ()).encrypt_history(true) });
    r.get("/logout-done", |_: ()| async { inertia::render("home", ()).clear_history() });
    r.get("/boom", |_: ()| async { (StatusCode::INTERNAL_SERVER_ERROR, inertia::render("errors/show", ())) });
    r.layer(inertia::layer().version("v1"));
    r.build(()).unwrap()
}

#[tokio::test]
async fn scroll_props_for_infinite_scroll() {
    let app = log_app();
    let json = send(&app, Method::GET, "/log?page=2", &[INERTIA, V1]).await.2;
    assert!(json.contains(r#""scrollProps":{"lines":{"pageName":"page","previousPage":1,"nextPage":3,"currentPage":2,"reset":false}}"#), "{json}");
    assert!(json.contains(r#""mergeProps":["lines"]"#) && json.contains(r#""lines":[20,21]"#), "{json}");

    // Scrolling back up asks to prepend.
    let up = [INERTIA, V1, ("x-inertia-partial-component", "log"), ("x-inertia-partial-data", "lines"), ("x-inertia-infinite-scroll-merge-intent", "prepend")];
    let json = send(&app, Method::GET, "/log?page=1", &up).await.2;
    assert!(json.contains(r#""prependProps":["lines"]"#) && !json.contains("mergeProps") && json.contains(r#""nextPage":2"#), "{json}");

    // Reset: no merge label, scrollProps says so.
    let reset = [INERTIA, V1, ("x-inertia-partial-component", "log"), ("x-inertia-partial-data", "lines"), ("x-inertia-reset", "lines")];
    let json = send(&app, Method::GET, "/log?page=1", &reset).await.2;
    assert!(json.contains(r#""reset":true"#) && !json.contains("mergeProps") && !json.contains("prependProps"), "{json}");
}

#[tokio::test]
async fn history_flags_and_real_statuses() {
    let app = log_app();
    let json = send(&app, Method::GET, "/secret", &[INERTIA, V1]).await.2;
    assert!(json.contains(r#""encryptHistory":true"#) && !json.contains("clearHistory"), "{json}");
    let json = send(&app, Method::GET, "/logout-done", &[INERTIA, V1]).await.2;
    assert!(json.contains(r#""clearHistory":true"#) && !json.contains("encryptHistory"), "{json}");
    let json = send(&app, Method::GET, "/log", &[INERTIA, V1]).await.2;
    assert!(!json.contains("History"), "flags are omitted when false: {json}");

    // A handler's own 500 stays a 500 and still renders the page.
    let (status, _, json) = send(&app, Method::GET, "/boom", &[INERTIA, V1]).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(json.contains(r#""component":"errors/show""#), "{json}");

    // Without the layer, a page is a loud 599.
    let mut r = Router::new();
    r.get("/", |_: ()| async { inertia::render("home", ()) });
    assert_eq!(send(&r.build(()).unwrap(), Method::GET, "/", &[]).await.0.as_u16(), 599);
}
