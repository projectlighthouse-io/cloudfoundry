#![allow(clippy::unwrap_used)] // test
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use route::{Bind, BoxError, Bound, Router, empty};

struct App {
    machines: Vec<(&'static str, &'static str)>,
}

struct Machine {
    name: &'static str,
}

/// A stand-in for a uuid: only lowercase hex parses.
struct Hex(String);

impl std::str::FromStr for Hex {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        if s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase() && !c.is_ascii_digit()) { Ok(Hex(s.into())) } else { Err(()) }
    }
}

impl Bind<App> for Machine {
    const PARAM: &'static str = "machine";
    type Key = Hex;
    async fn resolve(key: Hex, app: &App) -> Result<Option<Self>, BoxError> {
        if key.0 == "dead" {
            return Err("store down".into());
        }
        Ok(app.machines.iter().find(|(k, _)| *k == key.0).map(|(_, name)| Machine { name }))
    }
}

async fn show(Bound(m): Bound<Machine>) -> &'static str {
    m.name
}

#[tokio::test]
async fn bound_resolves_the_named_param() {
    let mut r = Router::new();
    r.get("/racks/{rack}/machines/{machine}", show);
    let routes = r.build(App { machines: vec![("abc", "builder")] }).unwrap();
    let get = |uri: &'static str| {
        let routes = routes.clone();
        async move {
            let res = routes.handle(Request::get(uri).body(empty()).unwrap()).await;
            (res.status(), String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap())
        }
    };
    assert_eq!(get("/racks/1/machines/abc").await, (StatusCode::OK, "builder".into()));
    assert_eq!(get("/racks/1/machines/42").await.0, StatusCode::NOT_FOUND, "unparsable key");
    assert_eq!(get("/racks/1/machines/bee").await.0, StatusCode::NOT_FOUND, "unknown key");
    assert_eq!(get("/racks/1/machines/dead").await.0, StatusCode::INTERNAL_SERVER_ERROR);
}
