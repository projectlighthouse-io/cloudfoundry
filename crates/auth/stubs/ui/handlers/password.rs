//! Published by `art publish auth --with-ui`. App code: change it freely.

use auth::{Auth, ChangeError};
use route::{Body, Cx, IntoResponse, Json, http::{Response, StatusCode}};
use serde::Serialize;
use validate::{Errors, Valid};

use crate::{requests::password::ChangePassword, state::App, users::{User, Users}};

#[derive(Serialize)]
pub struct PasswordPage {
    status: Option<&'static str>,
}

pub async fn edit(_: ()) -> inertia::Page<PasswordPage> {
    inertia::render("settings/password", PasswordPage { status: None })
}

/// Check the current password, store the new one. The session gets a new id
/// and "remember me" cookies stop working; the user stays logged in here.
pub async fn update(cx: Cx<App>, auth: Auth<User>, input: Valid<Json<ChangePassword>>) -> Result<inertia::Page<PasswordPage>, Response<Body>> {
    let users = Users { db: cx.db.clone() };
    match auth.change_password(&users, &input.current_password, &input.password).await {
        Ok(()) => Ok(inertia::render("settings/password", PasswordPage { status: Some("Your password has been changed.") })),
        Err(ChangeError::WrongPassword) => {
            let mut errors = Errors::new();
            errors.add("current_password", "The current password is incorrect.");
            Err(errors.into_response())
        }
        Err(ChangeError::NoPassword) => {
            let mut errors = Errors::new();
            errors.add("current_password", "This account has no password yet. Use “Forgot password” to set one.");
            Err(errors.into_response())
        }
        Err(ChangeError::NotLoggedIn) => Err(StatusCode::UNAUTHORIZED.into_response()),
        Err(ChangeError::Other(e)) => {
            tracing::error!("change password: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}
