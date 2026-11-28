//! Published by `art publish auth`. App code: change it freely.
//!
//! Changing the email address: the new address gets a verification link and
//! replaces the old one only once it is verified.

use auth::RecentlyConfirmed;
use route::{Body, Cx, Json, http::{Response, StatusCode}};
use validate::Valid;

use crate::{
    handlers::verification::{failed, send_verification_link},
    requests::email::ChangeEmail,
    state::App,
    users::{User, UserStore},
};

/// Needs a recent confirmation (`423` until then): `204`, or `422`.
pub async fn update(cx: Cx<App>, user: RecentlyConfirmed<User>, input: Valid<Json<ChangeEmail>>) -> Result<StatusCode, Response<Body>> {
    let users = UserStore { db: cx.db.clone() };
    let email = input.email.trim();
    let token = auth::verification_token(&users, &user, Some(email)).await.map_err(failed)?;
    send_verification_link(&cx, email, &token).await;
    Ok(StatusCode::NO_CONTENT)
}
