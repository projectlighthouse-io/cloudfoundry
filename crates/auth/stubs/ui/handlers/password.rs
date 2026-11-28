//! Published by `art publish auth --with-ui`. App code: change it freely.

use auth::{Auth, Authed, ChangeError};
use route::{Body, Cx, IntoResponse, Json, http::{Response, StatusCode}};
use serde::Serialize;
use validate::{Errors, Valid};

use crate::{requests::password::ChangePassword, state::App, users::{User, Users}};

#[derive(Serialize)]
pub struct PasswordPage {
    email: String,
    status: Option<&'static str>,
}

pub async fn edit(user: Authed<User>) -> inertia::Page<PasswordPage> {
    inertia::render("settings/password", PasswordPage { email: user.email.clone(), status: None })
}

/// Check the current password, store the new one. The session gets a new id
/// and "remember me" cookies stop working; the user stays logged in here.
pub async fn update(cx: Cx<App>, auth: Auth<User>, input: Valid<Json<ChangePassword>>) -> Result<inertia::Page<PasswordPage>, Response<Body>> {
    let users = Users { db: cx.db.clone() };
    match auth.change_password(&users, &input.current_password, &input.password).await {
        Ok(()) => {
            let email = auth.user().map(|u| u.email.clone()).unwrap_or_default();
            Ok(inertia::render("settings/password", PasswordPage { email, status: Some("Your password has been changed.") }))
        }
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
