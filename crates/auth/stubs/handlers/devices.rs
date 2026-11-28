//! Published by `art publish auth`. App code: change it freely.
//!
//! Browsers & devices: the user's sessions, from the sessions table.
//! Ending sessions needs a recent confirmation, so a stolen session cannot
//! lock the owner out.

use auth::{Auth, Authed, RecentlyConfirmed};
use route::{Body, Cx, IntoResponse, Json, Path, http::{Response, StatusCode}};
use serde::Serialize;
use session::{Device, Session, Uuid};

use crate::{state::App, users::{User, UserStore}};

type Result<T> = std::result::Result<T, Response<Body>>;

fn server_error(e: impl std::fmt::Display) -> Response<Body> {
    tracing::error!("devices: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

#[derive(Serialize)]
pub struct Devices {
    /// This browser's `public_id`.
    current: Option<Uuid>,
    devices: Vec<Device>,
}

pub async fn index(cx: Cx<App>, user: Authed<User>, session: Session) -> Result<Json<Devices>> {
    let devices = session::Devices::devices(&UserStore { db: cx.db.clone() }.sessions(), user.id).await.map_err(server_error)?;
    Ok(Json(Devices { current: session.public_id(), devices }))
}

/// Log one browser out: `204`, or `404` if it is not the user's.
pub async fn destroy(cx: Cx<App>, auth: Auth<User>, _: RecentlyConfirmed<User>, Path(device): Path<Uuid>) -> Result<StatusCode> {
    match auth.revoke_session(&UserStore { db: cx.db.clone() }.sessions(), device).await.map_err(server_error)? {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(StatusCode::NOT_FOUND.into_response()),
    }
}

/// Log out everywhere else.
pub async fn destroy_others(cx: Cx<App>, auth: Auth<User>, _: RecentlyConfirmed<User>) -> Result<StatusCode> {
    auth.revoke_other_sessions(&UserStore { db: cx.db.clone() }.sessions()).await.map_err(server_error)?;
    Ok(StatusCode::NO_CONTENT)
}
