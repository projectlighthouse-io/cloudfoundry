use http::{HeaderValue, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use route::{Body, BuildError, Cx, Json, Path, Redirect, Router, Routes, UrlError, body, empty};
use serde::Deserialize;

struct App {
    greeting: &'static str,
}

#[derive(Deserialize)]
struct RunId(u64);

#[derive(Deserialize)]
struct Pair {
    a: String,
    b: u32,
}

#[derive(Deserialize)]
struct New {
    title: String,
}

async fn home(cx: Cx<App>) -> &'static str {
    cx.greeting
}
async fn show(Path(RunId(id)): Path<RunId>) -> String {
    format!("run {id}")
}
async fn pair(Path(p): Path<Pair>) -> String {
    format!("{}-{}", p.a, p.b)
}
async fn store(_: Cx<App>, Json(n): Json<New>) -> Redirect {
    Redirect::to_route("runs.show").params(&[("id", &n.title)])
}
async fn bad_redirect(_: ()) -> Redirect {
    Redirect::to_route("nope")
}

fn routes() -> Routes {
    let mut r = Router::new();
    r.get("/", home).name("home");
    r.get("/bad", bad_redirect);
    r.group("/runs", |r| {
        r.name_prefix("runs.");
        r.layer(tower::util::MapResponseLayer::new(|mut res: http::Response<Body>| {
            res.headers_mut().insert("x-group", HeaderValue::from_static("1"));
            res
        }));
        r.get("/{id}", show).name("show");
        r.post("/", store).name("store");
        r.get("/{a}/{b}", pair);
    });
    r.build(App { greeting: "hi" }).unwrap()
}

async fn call(routes: &Routes, method: Method, uri: &str, b: Body) -> (StatusCode, http::HeaderMap, String) {
    let req = Request::builder().method(method).uri(uri).header(header::CONTENT_TYPE, "application/json").body(b).unwrap();
    let res = routes.handle(req).await;
    let (parts, b) = res.into_parts();
    let text = String::from_utf8(b.collect().await.unwrap().to_bytes().to_vec()).unwrap();
    (parts.status, parts.headers, text)
}

#[tokio::test]
async fn dispatch() {
    let r = routes();
    assert_eq!(call(&r, Method::GET, "/", empty()).await.2, "hi");
    assert_eq!(call(&r, Method::GET, "/runs/42", empty()).await.2, "run 42");
    assert_eq!(call(&r, Method::GET, "/runs/a%20b/7", empty()).await.2, "a b-7");
    assert_eq!(call(&r, Method::GET, "/runs/x", empty()).await.0, StatusCode::NOT_FOUND, "bad typed param");
    assert_eq!(call(&r, Method::GET, "/missing", empty()).await.0, StatusCode::NOT_FOUND);

    let (status, headers, _) = call(&r, Method::DELETE, "/", empty()).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(headers[header::ALLOW], "GET");

    // Group layer applies inside the group only.
    assert_eq!(call(&r, Method::GET, "/runs/1", empty()).await.1["x-group"], "1");
    assert!(call(&r, Method::GET, "/", empty()).await.1.get("x-group").is_none());
}

#[tokio::test]
async fn named_redirects_resolve_inside_layers() {
    let r = routes();
    let (status, headers, _) = call(&r, Method::POST, "/runs", body(r#"{"title":"9"}"#)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/runs/9");
    assert_eq!(call(&r, Method::GET, "/bad", empty()).await.0, StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn urls() {
    let r = routes();
    assert_eq!(r.url("runs.show", &[("id", "a/b")]).unwrap(), "/runs/a%2Fb");
    assert_eq!(r.url("home", &[]).unwrap(), "/");
    assert_eq!(r.url("runs.show", &[]), Err(UrlError::MissingParam("id".into())));
    assert_eq!(r.url("home", &[("x", "1")]), Err(UrlError::ExtraParam("x".into())));
    assert_eq!(r.url("nope", &[]), Err(UrlError::UnknownName("nope".into())));

    let show = r.table().find(|i| i.name.as_deref() == Some("runs.show")).unwrap();
    assert_eq!(show.path, "/runs/{id}");
    assert_eq!(show.layers.len(), 1);
}

#[test]
fn bad_tables_do_not_build() {
    let mut r = Router::<()>::new();
    r.get("/a", |_: ()| async {}).name("x");
    r.get("/b", |_: ()| async {}).name("x");
    assert!(matches!(r.build(()), Err(BuildError::DuplicateName(_))));

    let mut r = Router::<()>::new();
    r.get("/a", |_: ()| async {});
    r.get("/a", |_: ()| async {});
    assert!(matches!(r.build(()), Err(BuildError::DuplicateRoute(..))));

    let mut r = Router::<()>::new();
    r.get("/{a}", |_: ()| async {});
    r.get("/{b}", |_: ()| async {});
    assert!(matches!(r.build(()), Err(BuildError::Conflict { .. })));

    #[derive(Clone, Copy)]
    enum Name {
        Home,
        Gone,
    }
    impl route::Name for Name {
        const ALL: &'static [Self] = &[Self::Home, Self::Gone];
        fn as_str(&self) -> &'static str {
            match self {
                Self::Home => "home",
                Self::Gone => "gone",
            }
        }
    }
    let mut r = Router::<()>::new();
    r.get("/", |_: ()| async {}).name(Name::Home);
    assert!(matches!(r.build_named::<Name>(()), Err(BuildError::UnregisteredName("gone"))));
}

#[tokio::test]
async fn fallback_serves_unmatched_paths() {
    let mut r = Router::<()>::new();
    r.get("/", |_: ()| async { "home" });
    r.fallback(tower::service_fn(|_: Request<Body>| async { Ok::<_, std::convert::Infallible>(http::Response::new(body("static"))) }));
    let routes = r.build(()).unwrap();
    assert_eq!(call(&routes, Method::GET, "/app.js", empty()).await.2, "static");
    assert_eq!(call(&routes, Method::GET, "/", empty()).await.2, "home");
}

#[tokio::test]
async fn group_prefix_with_trailing_slash() {
    let mut r = Router::<()>::new();
    r.group("/", |r| {
        r.get("/go", |_: ()| async { "go" }).name("go");
        r.group("/api/", |r| {
            r.get("/x", |_: ()| async { "x" }).name("x");
            r.get("/", |_: ()| async { "api" }).name("api");
        });
    });
    let routes = r.build(()).unwrap();
    assert_eq!(routes.url("go", &[]).unwrap(), "/go");
    assert_eq!(routes.url("x", &[]).unwrap(), "/api/x");
    assert_eq!(routes.url("api", &[]).unwrap(), "/api");
    assert_eq!(call(&routes, Method::GET, "/go", empty()).await.2, "go");
}
