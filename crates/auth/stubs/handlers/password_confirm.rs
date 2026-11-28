//! Published by `art publish auth`. App code: change it freely.
//!
//! Sudo mode: routes whose handlers take `auth::PasswordConfirmed<User>` answer
//! `423 Locked` until the user confirms their password here.

use auth::{Auth, ChangeError};
use route::{Body, Cx, IntoResponse, Json, http::{Response, StatusCode}};
use validate::{Errors, Valid};

use crate::{requests::password_confirm::ConfirmPassword, state::App, users::{User, Users}};

/// `204` once confirmed (valid for `auth::Config::password_timeout`); `422` when wrong.
pub async fn store(cx: Cx<App>, auth: Auth<User>, input: Valid<Json<ConfirmPassword>>) -> Result<StatusCode, Response<Body>> {
    let users = Users { db: cx.db.clone() };
    match auth.confirm_password(&users, &input.password).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
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
