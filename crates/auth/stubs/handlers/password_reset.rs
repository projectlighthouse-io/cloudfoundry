//! Published by `art publish auth`. App code: change it freely.

use auth::{Auth, Remember, ResetError};
use route::{Body, Cx, IntoResponse, Json, http::{Response, StatusCode}};
use serde::Serialize;
use validate::{Errors, Valid};

use crate::{requests::password_reset::{ForgotPassword, ResetPassword}, state::App, users::{User, Users}};

#[derive(Serialize)]
pub struct Status {
    status: &'static str,
}

/// Always the same answer, so the endpoint does not reveal who has an account.
pub async fn email(cx: Cx<App>, input: Valid<Json<ForgotPassword>>) -> Result<Json<Status>, Response<Body>> {
    let users = Users { db: cx.db.clone() };
    match auth::reset_link_token(&users, &input.email).await {
        Ok((user, token)) => send_reset_link(&cx.cfg.app_url, &user, &token),
        Err(ResetError::UnknownUser | ResetError::Throttled) => {}
        Err(e) => return Err(server_error(e)),
    }
    Ok(Json(Status { status: "If that email has an account, a reset link is on its way." }))
}

/// Replace with your mailer. The link points at your frontend's reset page
/// (`app_url` in config) and works for an hour (`auth::RESET_EXPIRES`).
fn send_reset_link(base: &str, user: &User, token: &str) {
    let email: String = user
        .email
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~@".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    // Logging the link is for local development only: it lets anyone who reads the log reset the password.
    tracing::info!("password reset link for {}: {base}/reset-password/{token}?email={email}", user.email);
}

/// Set the new password, sign out "remember me" devices, and log in: `204`.
pub async fn update(cx: Cx<App>, auth: Auth<User>, input: Valid<Json<ResetPassword>>) -> Result<StatusCode, Response<Body>> {
    let users = Users { db: cx.db.clone() };
    match auth::reset_password(&users, &input.email, &input.token, &input.password).await {
        Ok(user) => {
            users.set_remember_token(user.id, None).await.map_err(server_error)?;
            auth.login(&user);
            Ok(StatusCode::NO_CONTENT)
        }
        Err(ResetError::InvalidToken | ResetError::UnknownUser) => {
            let mut errors = Errors::new();
            errors.add("email", "This password reset link is invalid or has expired.");
            Err(errors.into_response())
        }
        Err(e) => Err(server_error(e)),
    }
}

fn server_error(e: impl std::fmt::Display) -> Response<Body> {
    tracing::error!("password reset: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}
