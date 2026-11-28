//! Published by `art publish auth --with-ui`. App code: change it freely.
//!
//! Changing the email address: the new address gets a verification link and
//! replaces the old one only once it is verified.

use auth::{Authed, RecentlyConfirmed};
use route::{Body, Cx, Json, Redirect, http::Response};
use serde::Serialize;
use session::Session;
use ts_rs::TS;
use validate::Valid;

use crate::{
    handlers::verification::{STATUS, failed, send_verification_link},
    requests::email::ChangeEmail,
    state::App,
    users::{User, UserStore},
};

/// Props for `resources/js/pages/settings/email.vue`.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct EmailPage {
    email: String,
    verified: bool,
    status: Option<String>,
}

pub async fn edit(user: Authed<User>, session: Session) -> inertia::Page<EmailPage> {
    let status = session.get(STATUS).ok().flatten();
    inertia::render("settings/email", EmailPage { email: user.email.clone(), verified: user.email_verified_at.is_some(), status })
}

/// Needs a recent confirmation: the user is sent to confirm first.
pub async fn update(cx: Cx<App>, user: RecentlyConfirmed<User>, session: Session, input: Valid<Json<ChangeEmail>>) -> Result<Redirect, Response<Body>> {
    let users = UserStore { db: cx.db.clone() };
    let email = input.email.trim();
    let token = auth::verification_token(&users, &user, Some(email)).await.map_err(failed)?;
    send_verification_link(&cx, email, &token).await;
    if let Err(e) = session.flash(STATUS, &format!("We sent a link to {email}. Your address changes once you open it.")) {
        tracing::error!("flash: {e}");
    }
    Ok(Redirect::to_route("email.edit"))
}
