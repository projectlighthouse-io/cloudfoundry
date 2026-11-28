//! Published by `art publish auth --with-ui`. App code: change it freely.
//!
//! Browsers & devices: the user's sessions, from the sessions table.
//! Ending sessions needs a recent confirmation, so a stolen session cannot
//! lock the owner out.

use auth::{Auth, Authed, RecentlyConfirmed};
use route::{Body, Cx, IntoResponse, Path, Redirect, http::{Response, StatusCode}};
use serde::Serialize;
use session::{Device, Session, Uuid};
use ts_rs::TS;

use crate::{state::App, users::{User, UserStore}};

type Result<T> = std::result::Result<T, Response<Body>>;

const DEVICES: &str = "/settings/devices";

/// Props for `resources/js/pages/settings/devices.vue`.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct DevicesPage {
    email: String,
    /// This browser's `public_id`.
    #[ts(type = "string | null")]
    current: Option<Uuid>,
    devices: Vec<Device>,
}

fn server_error(e: impl std::fmt::Display) -> Response<Body> {
    tracing::error!("devices: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

pub async fn index(cx: Cx<App>, user: Authed<User>, session: Session) -> Result<inertia::Page<DevicesPage>> {
    let devices = session::Devices::devices(&UserStore { db: cx.db.clone() }.sessions(), user.id).await.map_err(server_error)?;
    Ok(inertia::render("settings/devices", DevicesPage { email: user.email.clone(), current: session.public_id(), devices }))
}

/// Log one browser out.
pub async fn destroy(cx: Cx<App>, auth: Auth<User>, _: RecentlyConfirmed<User>, Path(device): Path<Uuid>) -> Result<Redirect> {
    match auth.revoke_session(&UserStore { db: cx.db.clone() }.sessions(), device).await.map_err(server_error)? {
        true => Ok(Redirect::to(DEVICES)),
        false => Err(StatusCode::NOT_FOUND.into_response()),
    }
}

/// Log out everywhere else.
pub async fn destroy_others(cx: Cx<App>, auth: Auth<User>, _: RecentlyConfirmed<User>) -> Result<Redirect> {
    auth.revoke_other_sessions(&UserStore { db: cx.db.clone() }.sessions()).await.map_err(server_error)?;
    Ok(Redirect::to(DEVICES))
}
