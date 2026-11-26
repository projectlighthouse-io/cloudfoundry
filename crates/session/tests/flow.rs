#![cfg(all(feature = "route", feature = "csrf"))]

use std::sync::Arc;

use http_body_util::BodyExt;
use route::{
    Router, Routes, body, empty,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use securecookie::{Codec, Keys};
use session::{MemoryStore, Session, Store};

async fn count(s: Session) -> String {
    let n = s.get::<u32>("n").unwrap().unwrap_or(0) + 1;
    s.insert("n", &n).unwrap();
    n.to_string()
}

async fn hook(_: ()) -> &'static str {
    "hooked"
}

fn app(store: MemoryStore) -> Routes {
    let mut r = Router::new();
    r.get("/", |_: ()| async { "home" });
    r.post("/count", count);
    r.post("/hook", hook).name("hook");
    r.layer(session::layer(StoreRef(Arc::new(store)), Codec::new(Keys::parse(&Keys::generate()).unwrap())))
        .layer(csrf::layer::<Session>().except(["hook"]));
    r.build(()).unwrap()
}

/// Share one MemoryStore between the layer and the test.
#[derive(Clone)]
struct StoreRef(Arc<MemoryStore>);

impl Store for StoreRef {
    async fn load(&self, id: &session::SessionId) -> Result<Option<session::Record>, session::Error> {
        self.0.load(id).await
    }
    async fn save(&self, r: &session::Record) -> Result<(), session::Error> {
        self.0.save(r).await
    }
    async fn delete(&self, id: &session::SessionId) -> Result<(), session::Error> {
        self.0.delete(id).await
    }
}

/// A tiny cookie jar: send what was set, remember what comes back.
struct Client {
    routes: Routes,
    jar: Vec<(String, String)>,
}

impl Client {
    async fn send(&mut self, method: Method, uri: &str, xsrf: bool) -> (StatusCode, String) {
        let mut req = Request::builder().method(method).uri(uri);
        if !self.jar.is_empty() {
            let c: Vec<String> = self.jar.iter().map(|(k, v)| format!("{k}={v}")).collect();
            req = req.header(header::COOKIE, c.join("; "));
        }
        if xsrf {
            req = req.header("x-xsrf-token", self.get("XSRF-TOKEN").unwrap_or_default());
        }
        let res = self.routes.handle(req.body(empty()).unwrap()).await;
        self.remember(res.headers());
        let status = res.status();
        let text = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        (status, text)
    }

    fn remember(&mut self, headers: &HeaderMap) {
        for v in headers.get_all(header::SET_COOKIE) {
            let (k, v) = v.to_str().unwrap().split(';').next().unwrap().split_once('=').unwrap();
            self.jar.retain(|(name, _)| name != k);
            self.jar.push((k.to_owned(), v.to_owned()));
        }
    }

    fn get(&self, name: &str) -> Option<String> {
        self.jar.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }
}

#[tokio::test]
async fn session_and_csrf_round_trip() {
    let _ = body(""); // keep the import used across feature sets
    let mut c = Client { routes: app(MemoryStore::new()), jar: vec![] };

    assert_eq!(c.send(Method::GET, "/", false).await.0, StatusCode::OK);
    assert!(c.get("XSRF-TOKEN").is_some() && c.get("session").is_some(), "token issued on first visit");

    assert_eq!(c.send(Method::POST, "/count", false).await.0.as_u16(), 419, "no header");
    assert_eq!(c.send(Method::POST, "/count", true).await, (StatusCode::OK, "1".into()));
    assert_eq!(c.send(Method::POST, "/count", true).await, (StatusCode::OK, "2".into()), "session persisted");

    let mut fresh = Client { routes: c.routes.clone(), jar: vec![] };
    assert_eq!(fresh.send(Method::POST, "/hook", false).await.1, "hooked", "exempt route");
}
