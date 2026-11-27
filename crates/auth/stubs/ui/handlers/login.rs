//! Published by `art publish auth --with-ui`. App code: change it freely.

use auth::{AttemptError, Auth};
use route::{Body, Cx, IntoResponse, Json, Redirect, http::{Response, StatusCode}};
use validate::{Errors, Valid};

use crate::{requests::login::Login, state::App, users::{User, Users}};

pub async fn show(_: ()) -> inertia::Page<()> {
    inertia::render("auth/login", ())
}

/// `auth.attempt` throttles, checks the password, upgrades old hashes and logs
/// in. Add your own checks around it (banned users, audit log, ...).
pub async fn login(cx: Cx<App>, auth: Auth<User>, input: Valid<Json<Login>>) -> Result<Redirect, Response<Body>> {
    let users = Users { db: cx.db.clone() };
    let message = match auth.attempt(&users, &input.email, &input.password, input.remember).await {
        Ok(_user) => return Ok(Redirect::to_route("home")),
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

pub async fn logout(auth: Auth<User>) -> Redirect {
    auth.logout();
    Redirect::to_route("home")
}
