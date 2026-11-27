#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
#![cfg(feature = "session")]

mod common;

use common::TeamRole;
use http_body_util::BodyExt;
use route::{
    Redirect, Router, Routes,
    http::{Method, Request, header, request::Parts},
};
use securecookie::{Codec, Keys};
use teams::{CurrentTeam, Membership, Teams};

fn user(parts: &Parts) -> Option<i64> {
    parts.headers.get("x-user")?.to_str().ok()?.parse().ok()
}

async fn current(c: CurrentTeam<TeamRole>) -> String {
    c.as_ref().map_or("none".into(), |m| m.team.name.clone())
}

async fn switch((m, mut c): (Membership<TeamRole>, CurrentTeam<TeamRole>)) -> Redirect {
    c.set(&m);
    Redirect::to("/current")
}

/// Cookies sent back, as one jar per user.
struct Client {
    routes: Routes,
    user: i64,
    cookie: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, uri: &str) -> String {
        let mut req = Request::builder().method(method).uri(uri).header("x-user", self.user.to_string());
        if let Some(c) = &self.cookie {
            req = req.header(header::COOKIE, c);
        }
        let res = self.routes.handle(req.body(route::empty()).unwrap()).await;
        if let Some(set) = res.headers().get(header::SET_COOKIE) {
            self.cookie = Some(set.to_str().unwrap().split(';').next().unwrap().to_owned());
        }
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
    }
}

#[tokio::test]
async fn current_team_falls_back_and_follows_switches() {
    let t = Teams::new(common::Store::new());
    let first = t.create_team(1, None, "First").await.unwrap();
    let second = t.create_team(1, None, "Second").await.unwrap();
    let other = t.create_team(2, None, "Other").await.unwrap();

    let mut r = Router::new();
    r.get("/current", current);
    r.post("/teams/{team}/switch", switch);
    r.layer(session::layer(session::MemoryStore::new(), Codec::new(Keys::parse(&Keys::generate()).unwrap())))
        .layer(teams::layer(t.clone(), user));
    let routes = r.build(()).unwrap();

    let mut ada = Client { routes: routes.clone(), user: 1, cookie: None };
    assert_eq!(ada.send(Method::GET, "/current").await, "First", "the first team by id");
    ada.send(Method::POST, &format!("/teams/{}/switch", second.id)).await;
    assert_eq!(ada.send(Method::GET, "/current").await, "Second");
    ada.send(Method::POST, &format!("/teams/{}/switch", other.id)).await;
    assert_eq!(ada.send(Method::GET, "/current").await, "Second", "cannot switch to a team you cannot open");

    // Losing access drops the choice.
    t.add_member(&second, 2, TeamRole::Owner).await.unwrap();
    t.remove_member(&second, 1).await.unwrap();
    assert_eq!(ada.send(Method::GET, "/current").await, "First");
    t.delete_team(&first).await.unwrap();
    assert_eq!(ada.send(Method::GET, "/current").await, "none");
}
