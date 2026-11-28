//! Published by `art publish auth --with-ui`. App code: change it freely.

use auth::{Auth, Remember, ResetError};
use route::{Body, Cx, IntoResponse, Json, Path, Query, Redirect, http::{Response, StatusCode}};
use serde::{Deserialize, Serialize};
use validate::{Errors, Valid};

use crate::{mailer::mailer, requests::password_reset::{ForgotPassword, ResetPassword}, state::App, users::{User, UserStore}};

#[derive(Serialize)]
pub struct Forgot {
    status: Option<&'static str>,
}

pub async fn request(_: ()) -> inertia::Page<Forgot> {
    inertia::render("auth/forgot-password", Forgot { status: None })
}

/// Always the same answer, so the form does not reveal who has an account.
pub async fn email(cx: Cx<App>, input: Valid<Json<ForgotPassword>>) -> Result<inertia::Page<Forgot>, Response<Body>> {
    let users = UserStore { db: cx.db.clone() };
    match auth::reset_link_token(&users, &input.email).await {
        Ok((user, token)) => send_reset_link(&cx, &user, &token).await,
        Err(ResetError::UnknownUser | ResetError::Throttled) => {}
        Err(e) => return Err(server_error(e)),
    }
    let status = Some("If that email has an account, a reset link is on its way.");
    Ok(inertia::render("auth/forgot-password", Forgot { status }))
}

/// Mail the link; it works for an hour (`auth::RESET_EXPIRES`). A mail
/// failure is logged; the user can ask again.
async fn send_reset_link(app: &App, user: &User, token: &str) {
    let email: String = user
        .email
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~@".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    let link = format!("{}/reset-password/{token}?email={email}", app.cfg.app_url);
    let text = format!("Someone asked to reset the password of your account.\n\nChoose a new one here (the link works for an hour):\n{link}\n\nIf it was not you, ignore this mail.");
    if let Err(e) = mailer(app).send(mail::Message::new(&user.email, "Reset your password", text)).await {
        tracing::error!("password reset mail to {}: {e}", user.email);
    }
}

#[derive(Deserialize)]
pub struct EmailQuery {
    #[serde(default)]
    email: String,
}

#[derive(Serialize)]
pub struct ResetForm {
    token: String,
    email: String,
}

pub async fn edit(Path(token): Path<String>, Query(q): Query<EmailQuery>) -> inertia::Page<ResetForm> {
    inertia::render("auth/reset-password", ResetForm { token, email: q.email })
}

/// Set the new password, sign out "remember me" devices, and log in.
pub async fn update(cx: Cx<App>, auth: Auth<User>, input: Valid<Json<ResetPassword>>) -> Result<Redirect, Response<Body>> {
    let users = UserStore { db: cx.db.clone() };
    match auth::reset_password(&users, &input.email, &input.token, &input.password).await {
        Ok(user) => {
            users.set_remember_token(user.id, None).await.map_err(server_error)?;
            auth.login(&user);
            Ok(Redirect::to_route("home"))
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
