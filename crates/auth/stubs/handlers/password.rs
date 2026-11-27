//! Published by `art publish auth`. App code: change it freely.

use auth::{Auth, ChangeError};
use route::{Body, Cx, IntoResponse, Json, http::{Response, StatusCode}};
use validate::{Errors, Valid};

use crate::{requests::password::ChangePassword, state::App, users::{User, Users}};

/// `204` once changed; `422` when the current password is wrong. The session
/// gets a new id and "remember me" cookies stop working.
pub async fn update(cx: Cx<App>, auth: Auth<User>, input: Valid<Json<ChangePassword>>) -> Result<StatusCode, Response<Body>> {
    let users = Users { db: cx.db.clone() };
    match auth.change_password(&users, &input.current_password, &input.password).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
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
