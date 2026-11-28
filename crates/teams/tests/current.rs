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
use teams::{CurrentTeam, Membership, Store, Teams};

fn user(parts: &Parts) -> Option<i64> {
    parts.headers.get("x-user")?.to_str().ok()?.parse().ok()
}

async fn current(c: CurrentTeam<TeamRole>) -> String {
    c.as_ref().map_or("none".into(), |m| m.team.name.clone())
}

async fn switch(m: Membership<TeamRole>, mut c: CurrentTeam<TeamRole>) -> Redirect {
    c.set(&m).await.unwrap();
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
    ada.send(Method::POST, &format!("/teams/{}/switch", second.uuid)).await;
    assert_eq!(ada.send(Method::GET, "/current").await, "Second");
    ada.send(Method::POST, &format!("/teams/{}/switch", other.uuid)).await;
    assert_eq!(ada.send(Method::GET, "/current").await, "Second", "cannot switch to a team you cannot open");
    assert_eq!(t.store().current_team_id(1).await.unwrap(), Some(second.id), "stored for the user");
    let mut ada_elsewhere = Client { routes: routes.clone(), user: 1, cookie: None };
    assert_eq!(ada_elsewhere.send(Method::GET, "/current").await, "Second", "a new session lands on the stored team");

    // Losing access drops the choice.
    t.add_member(&second, 2, TeamRole::Owner).await.unwrap();
    t.remove_member(&second, 1).await.unwrap();
    assert_eq!(ada.send(Method::GET, "/current").await, "First");
    assert_eq!(t.store().current_team_id(1).await.unwrap(), Some(first.id), "the fallback is stored");
    t.delete_team(&first).await.unwrap();
    assert_eq!(ada.send(Method::GET, "/current").await, "none");
}

#[tokio::test]
async fn required_teams_give_everyone_a_personal_team() {
    let t = Teams::new(common::Store::new()).require_team(|user| format!("Personal {user}"));
    let mut r = Router::new();
    r.get("/current", current);
    r.layer(session::layer(session::MemoryStore::new(), Codec::new(Keys::parse(&Keys::generate()).unwrap())))
        .layer(teams::layer(t.clone(), user));
    let mut cy = Client { routes: r.build(()).unwrap(), user: 3, cookie: None };

    assert_eq!(cy.send(Method::GET, "/current").await, "Personal 3", "created on first use");
    assert_eq!(cy.send(Method::GET, "/current").await, "Personal 3", "once");
    let teams = t.teams_of(3).await.unwrap();
    assert_eq!(teams.len(), 1);
    assert_eq!(t.store().current_team_id(3).await.unwrap(), Some(teams[0].team.id));

    let personal = teams[0].team.clone();
    t.add_member(&personal, 4, TeamRole::Owner).await.unwrap();
    assert!(matches!(t.remove_member(&personal, 3).await, Err(teams::Error::LastTeam)), "cannot leave the last team");
    assert!(matches!(t.keeps_a_team(3, |team| team.id == personal.id).await, Err(teams::Error::LastTeam)), "nor delete it");
    let other = t.create_team(3, None, "Other").await.unwrap();
    t.remove_member(&personal, 3).await.unwrap();
    t.keeps_a_team(3, |team| team.id == personal.id).await.unwrap();
    assert_eq!(cy.send(Method::GET, "/current").await, "Other", "falls back to the team left");
    assert_eq!(t.store().current_team_id(3).await.unwrap(), Some(other.id));
}
