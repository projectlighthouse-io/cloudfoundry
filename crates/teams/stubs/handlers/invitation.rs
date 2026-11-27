//! Published by `art publish teams`. App code: change it freely.

use auth::Authed;
use route::{
    Body, Cx, Json, Path,
    http::Response,
};
use serde::Serialize;
use teams::Target;

use crate::{access, state::App, users::User};

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Joined {
    Team { id: i64 },
    Organization { id: i64 },
}

/// Accept an invitation as the logged-in user. Their email must be the invited one.
pub async fn accept(cx: Cx<App>, (user, Path(token)): (Authed<User>, Path<String>)) -> Result<Json<Joined>, Response<Body>> {
    let target = access::teams(&cx.db).accept(&token, user.id, &user.email).await.map_err(access::failed)?;
    Ok(Json(match target {
        Target::Team { team, .. } => Joined::Team { id: team },
        Target::Org { org, .. } => Joined::Organization { id: org },
    }))
}
