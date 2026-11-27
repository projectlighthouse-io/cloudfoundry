//! Published by `art publish auth`: JSON endpoints for your own
//! frontend, logged in by session cookie. App code: change it freely.

use auth::{AttemptError, Auth};
use route::{Body, Cx, IntoResponse, Json, http::{Response, StatusCode}};
use validate::{Errors, Valid};

use crate::{requests::login::Login, state::App, users::{User, Users}};

/// `204` when logged in; `422` with `{ "errors": { "email": [...] } }` when not.
pub async fn login(cx: Cx<App>, auth: Auth<User>, input: Valid<Json<Login>>) -> Result<StatusCode, Response<Body>> {
    let users = Users { db: cx.db.clone() };
    let message = match auth.attempt(&users, &input.email, &input.password, input.remember).await {
        Ok(_user) => return Ok(StatusCode::NO_CONTENT),
        Err(AttemptError::Failed) => "These credentials do not match our records.".to_owned(),
        Err(AttemptError::Throttled(wait)) => {
            format!("Too many login attempts. Please try again in {} seconds.", wait.as_secs().max(1))
        }
        Err(AttemptError::Other(e)) => {
            tracing::error!("login: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };
    let mut errors = Errors::new();
    errors.add("email", message);
    Err(errors.into_response())
}

pub async fn logout(auth: Auth<User>) -> StatusCode {
    auth.logout();
    StatusCode::NO_CONTENT
}
