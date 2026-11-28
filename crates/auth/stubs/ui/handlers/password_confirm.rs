//! Published by `art publish auth --with-ui`. App code: change it freely.
//!
//! Sudo mode: pages whose handlers take `auth::PasswordConfirmed<User>` send
//! the user here first, then back to where they were going.

use auth::{Auth, Authed, ChangeError};
use route::{Body, Cx, IntoResponse, Json, Redirect, http::{Response, StatusCode}};
use serde::Serialize;
use ts_rs::TS;
use validate::{Errors, Valid};

use crate::{requests::password_confirm::ConfirmPassword, state::App, users::{User, UserStore}};

/// Props for `resources/js/pages/auth/confirm-password.vue`.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct ConfirmPasswordPage {
    email: String,
}

pub async fn show(user: Authed<User>) -> inertia::Page<ConfirmPasswordPage> {
    inertia::render("auth/confirm-password", ConfirmPasswordPage { email: user.email.clone() })
}

/// Confirmed for `auth::Config::password_timeout`; back to the intended page.
pub async fn store(cx: Cx<App>, auth: Auth<User>, input: Valid<Json<ConfirmPassword>>) -> Result<Redirect, Response<Body>> {
    let users = UserStore { db: cx.db.clone() };
    match auth.confirm_password(&users, &input.password).await {
        Ok(()) => Ok(Redirect::to(auth.intended("/"))),
        Err(ChangeError::WrongPassword | ChangeError::NoPassword) => {
            let mut errors = Errors::new();
            errors.add("password", "The provided password is incorrect.");
            Err(errors.into_response())
        }
        Err(ChangeError::NotLoggedIn) => Err(StatusCode::UNAUTHORIZED.into_response()),
        Err(ChangeError::Other(e)) => {
            tracing::error!("confirm password: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}
