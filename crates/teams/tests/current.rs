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
use teams::{CurrentTeam, Membership, Store, Teams, require_current_team};

fn user(parts: &Parts) -> Option<i64> {
    parts.headers.get("x-user")?.to_str().ok()?.parse().ok()
}

async fn current(c: CurrentTeam<TeamRole>) -> String {
    c.as_ref().map_or("none".into(), |m| m.team.name.clone())
}

async fn switch(m: Membership<TeamRole>, (mut c, session): (CurrentTeam<TeamRole>, session::Session)) -> Redirect {
    c.set(&m).await.unwrap();
    Redirect::to(teams::intended(&session, "/current"))
}

/// Cookies sent back, as one jar per user.
struct Client {
    routes: Routes,
    user: i64,
    cookie: Option<String>,
}

impl Client {
    /// The body for 200, `"<status> <location>"` otherwise.
    async fn send(&mut self, method: Method, uri: &str) -> String {
        self.send_as(method, uri, false).await
    }

    async fn send_as(&mut self, method: Method, uri: &str, json: bool) -> String {
        let mut req = Request::builder().method(method).uri(uri).header("x-user", self.user.to_string());
        if json {
            req = req.header(header::ACCEPT, "application/json");
        }
        if let Some(c) = &self.cookie {
            req = req.header(header::COOKIE, c);
        }
        let res = self.routes.handle(req.body(route::empty()).unwrap()).await;
        if let Some(set) = res.headers().get(header::SET_COOKIE) {
            self.cookie = Some(set.to_str().unwrap().split(';').next().unwrap().to_owned());
        }
        let status = res.status();
        let location = res.headers().get(header::LOCATION).map(|v| v.to_str().unwrap().to_owned()).unwrap_or_default();
        let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        if status.is_success() { body } else { format!("{} {location}{body}", status.as_u16()) }
    }
}

/// `/current` and the switch are open; `/app/*` needs a current team; the
/// picker and "create a team" are outside the guard.
fn app(t: &Teams<common::Store>) -> Routes {
    let mut r = Router::new();
    r.get("/current", current);
    r.post("/teams/{team}/switch", switch);
    r.get("/teams/select", |_: ()| async { "picker" });
    r.get("/teams/create", |_: ()| async { "create a team" });
    r.group("/app", |r| {
        r.layer(require_current_team::<TeamRole>());
        r.get("/dashboard", |c: CurrentTeam<TeamRole>| async move { format!("dashboard of {}", c.as_ref().unwrap().team.name) });
    });
    r.layer(session::layer(session::MemoryStore::new(), Codec::new(Keys::parse(&Keys::generate()).unwrap())))
        .layer(teams::layer(t.clone(), user));
    r.build(()).unwrap()
}

#[tokio::test]
async fn picking_a_team_is_explicit() {
    let t = Teams::new(common::Store::new());
    t.create_team(1, None, "First").await.unwrap();
    let second = t.create_team(1, None, "Second").await.unwrap();
    let other = t.create_team(2, None, "Other").await.unwrap();
    let routes = app(&t);

    let mut ada = Client { routes: routes.clone(), user: 1, cookie: None };
    assert_eq!(ada.send(Method::GET, "/current").await, "none", "no silent fallback to the first team");
    assert_eq!(t.store().current_team_id(1).await.unwrap(), None);
    ada.send(Method::POST, &format!("/teams/{}/switch", second.uuid)).await;
    assert_eq!(ada.send(Method::GET, "/current").await, "Second");
    ada.send(Method::POST, &format!("/teams/{}/switch", other.uuid)).await;
    assert_eq!(ada.send(Method::GET, "/current").await, "Second", "cannot pick a team you cannot open");
    assert_eq!(t.store().current_team_id(1).await.unwrap(), Some(second.id), "stored for the user");
    let mut ada_elsewhere = Client { routes, user: 1, cookie: None };
    assert_eq!(ada_elsewhere.send(Method::GET, "/current").await, "Second", "a new session lands on the stored team");

    // Losing access clears the choice instead of picking another team.
    t.add_member(&second, 2, TeamRole::Owner).await.unwrap();
    t.remove_member(&second, 1).await.unwrap();
    assert_eq!(ada.send(Method::GET, "/current").await, "none");
    assert_eq!(t.store().current_team_id(1).await.unwrap(), None, "cleared");
}

#[tokio::test]
async fn require_current_team_blocks_until_one_is_picked() {
    let t = Teams::new(common::Store::new());
    let team = t.create_team(1, None, "Acme").await.unwrap();
    let routes = app(&t);
    let mut ada = Client { routes: routes.clone(), user: 1, cookie: None };

    assert_eq!(ada.send(Method::GET, "/app/dashboard?tab=1").await, "303 /teams/select", "page: to the picker");
    assert_eq!(ada.send_as(Method::GET, "/app/dashboard", true).await, r#"409 {"error":"team_required"}"#, "JSON: 409");
    assert_eq!(ada.send(Method::GET, "/teams/select").await, "picker", "the picker works without a team");
    assert_eq!(ada.send(Method::GET, "/teams/create").await, "create a team", "so does create");

    assert_eq!(ada.send(Method::POST, &format!("/teams/{}/switch", team.uuid)).await, "303 /app/dashboard?tab=1", "back to the page that asked");
    assert_eq!(ada.send(Method::GET, "/app/dashboard").await, "dashboard of Acme", "selected: passes, with the team");

    // Removed from the selected team: back to the picker (Ada has another team).
    let other = t.create_team(1, None, "Other").await.unwrap();
    t.add_member(&team, 2, TeamRole::Owner).await.unwrap();
    t.remove_member(&team, 1).await.unwrap();
    assert_eq!(ada.send(Method::GET, "/app/dashboard").await, "303 /teams/select");
    assert_eq!(t.store().current_team_id(1).await.unwrap(), None, "the stale pick is cleared");

    // No team at all: create one instead of picking.
    t.delete_team(&other).await.unwrap();
    assert_eq!(ada.send(Method::GET, "/app/dashboard").await, "303 /teams/create");
    let mut cy = Client { routes, user: 3, cookie: None };
    assert_eq!(cy.send(Method::GET, "/app/dashboard").await, "303 /teams/create", "a brand-new user");
}

#[tokio::test]
async fn paths_are_configurable_and_anonymous_requests_are_left_to_the_user_lookup() {
    let t = Teams::new(common::Store::new());
    t.create_team(1, None, "Acme").await.unwrap();
    let mut r = Router::new();
    r.group("/app", |r| {
        r.layer(require_current_team::<TeamRole>().select_path("/pick").create_path("/new-team"));
        r.get("/x", |_: CurrentTeam<TeamRole>| async { "x" });
    });
    r.layer(session::layer(session::MemoryStore::new(), Codec::new(Keys::parse(&Keys::generate()).unwrap())))
        .layer(teams::layer(t.clone(), user));
    let routes = r.build(()).unwrap();
    assert_eq!(Client { routes: routes.clone(), user: 1, cookie: None }.send(Method::GET, "/app/x").await, "303 /pick");
    assert_eq!(Client { routes: routes.clone(), user: 9, cookie: None }.send(Method::GET, "/app/x").await, "303 /new-team");
    let res = routes.handle(Request::builder().uri("/app/x").body(route::empty()).unwrap()).await;
    assert_eq!(res.status(), 401, "no user: the lookup's answer");
}
