//! Published by `art publish teams`. App code: change it freely.

use auth::Authed;
use route::{
    Body, Cx, IntoResponse, Json, Path,
    http::{Response, StatusCode},
};
use serde::Serialize;
use teams::{Org, Uuid, Store, Target, Team};

use crate::{access, state::App, users::User};

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Joined {
    Team { uuid: Uuid<Team> },
    Organization { uuid: Uuid<Org> },
}

/// Accept an invitation as the logged-in user. Their email must be the invited one.
pub async fn accept(cx: Cx<App>, user: Authed<User>, Path(token): Path<String>) -> Result<Json<Joined>, Response<Body>> {
    let t = access::teams(&cx.db);
    let joined = match t.accept(&token, user.id, &user.email).await.map_err(access::failed)? {
        Target::Team { team, .. } => t.store().team(team).await.map_err(access::failed)?.map(|team| Joined::Team { uuid: team.uuid }),
        Target::Org { org, .. } => t.store().org(org).await.map_err(access::failed)?.map(|org| Joined::Organization { uuid: org.uuid }),
    };
    joined.map(Json).ok_or_else(|| StatusCode::NOT_FOUND.into_response())
}
