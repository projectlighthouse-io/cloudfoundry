#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers

mod common;

use common::{DeleteTeam, InviteMember, ManageBilling, OrgRole, TeamRole};
use http_body_util::BodyExt;
use route::{
    Router,
    http::{Request, StatusCode, request::Parts},
};
use teams::{Can, Membership, OrgCan, OrgMembership, Uuid, Target, Teams};

/// The test's "logged-in user": an `x-user` header.
fn user(parts: &Parts) -> Option<i64> {
    parts.headers.get("x-user")?.to_str().ok()?.parse().ok()
}

async fn show(m: Membership<TeamRole>) -> String {
    format!("{} {:?} {}", m.team.name, m.role, m.via_org)
}

async fn invite(can: Can<InviteMember, TeamRole>) -> String {
    format!("invite {}", can.team.uuid)
}

async fn destroy(_: Can<DeleteTeam, TeamRole>) -> &'static str {
    "deleted"
}

async fn org(m: OrgMembership<OrgRole>) -> String {
    format!("{} {:?}", m.org.name, m.role)
}

async fn billing(_: OrgCan<ManageBilling, OrgRole>) -> &'static str {
    "billing"
}

#[tokio::test]
async fn inputs_check_membership_and_permissions() {
    let t = Teams::new(common::Store::new());
    let acme = t.create_org(1, "Acme").await.unwrap();
    t.add_org_member(&acme, 2, OrgRole::Admin).await.unwrap();
    t.add_org_member(&acme, 3, OrgRole::Member).await.unwrap();
    let core = t.create_team(1, Some(acme.id), "Core").await.unwrap();
    t.add_member(&core, 3, TeamRole::Admin).await.unwrap();
    let side = t.create_team(4, None, "Side").await.unwrap();

    let mut r = Router::new();
    r.layer(teams::layer(t.clone(), user));
    r.get("/teams/{team}", show);
    r.post("/teams/{team}/members/{member}", invite);
    r.delete("/teams/{team}", destroy);
    r.get("/orgs/{org}", org);
    r.get("/orgs/{org}/billing", billing);
    let routes = r.build(()).unwrap();
    let call = |method: &'static str, path: String, as_user: Option<i64>| {
        let routes = routes.clone();
        async move {
            let mut req = Request::builder().method(method).uri(path);
            if let Some(u) = as_user {
                req = req.header("x-user", u.to_string());
            }
            let res = routes.handle(req.body(route::empty()).unwrap()).await;
            let status = res.status();
            (status, String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap())
        }
    };
    let (c, s, a) = (core.uuid, side.uuid, acme.uuid);

    assert_eq!(call("GET", format!("/teams/{c}"), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call("GET", format!("/teams/{c}"), Some(1)).await.1, "Core Owner false");
    assert_eq!(call("GET", format!("/teams/{c}"), Some(2)).await.1, "Core Owner true", "org admin manages it");
    assert_eq!(call("GET", format!("/teams/{c}"), Some(3)).await.1, "Core Admin false");
    assert_eq!(call("GET", format!("/teams/{s}"), Some(1)).await.0, StatusCode::NOT_FOUND, "not a member: 404");
    assert_eq!(call("GET", format!("/teams/{}", core.id), Some(1)).await.0, StatusCode::NOT_FOUND, "internal id: 404");
    assert_eq!(call("GET", format!("/teams/{}", Uuid::<teams::Team>::new()), Some(1)).await.0, StatusCode::NOT_FOUND, "unknown uuid");
    assert_eq!(call("GET", "/teams/abc".into(), Some(1)).await.0, StatusCode::NOT_FOUND);

    // One path parameter among several.
    assert_eq!(call("POST", format!("/teams/{c}/members/9"), Some(3)).await.1, format!("invite {c}"));
    assert_eq!(call("DELETE", format!("/teams/{c}"), Some(3)).await.0, StatusCode::FORBIDDEN, "admins cannot delete");
    assert_eq!(call("DELETE", format!("/teams/{c}"), Some(2)).await.1, "deleted", "org admins act as owners");

    assert_eq!(call("GET", format!("/orgs/{a}"), Some(3)).await.1, "Acme Member");
    assert_eq!(call("GET", format!("/orgs/{a}"), Some(4)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call("GET", format!("/orgs/{}", acme.id), Some(1)).await.0, StatusCode::NOT_FOUND, "internal id: 404");
    assert_eq!(call("GET", format!("/orgs/{}", Uuid::<teams::Org>::new()), Some(1)).await.0, StatusCode::NOT_FOUND, "unknown uuid");
    assert_eq!(call("GET", format!("/orgs/{a}/billing"), Some(2)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(call("GET", format!("/orgs/{a}/billing"), Some(1)).await.1, "billing");
}

#[tokio::test]
async fn without_the_layer_inputs_fail_loudly() {
    let mut r = Router::new();
    r.get("/teams/{team}", show);
    let routes = r.build(()).unwrap();
    let res = routes.handle(Request::get("/teams/1").header("x-user", "1").body(route::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

/// Every key in a JSON value, at any depth.
fn keys(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, v) in m {
                out.push(k.clone());
                keys(v, out);
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|v| keys(v, out)),
        _ => {}
    }
}

#[tokio::test]
async fn json_carries_uuids_only() {
    let t = Teams::new(common::Store::new());
    let acme = t.create_org(1, "Acme").await.unwrap();
    let core = t.create_team(1, Some(acme.id), "Core").await.unwrap();
    let new = t.invite(Target::Team { team: core.id, role: TeamRole::Member }, "bo@example.com", 1, std::time::Duration::from_secs(60)).await.unwrap();
    let json = serde_json::json!({ "org": acme, "team": core, "invitation": new.invitation });
    let mut found = Vec::new();
    keys(&json, &mut found);
    assert!(!found.iter().any(|k| k == "id" || k.ends_with("_id") || k == "invited_by"), "internal ids leaked: {json}");
    assert_eq!(json["team"]["uuid"], core.uuid.to_string());
    assert_eq!(json["invitation"]["uuid"], new.invitation.uuid.to_string());
}
