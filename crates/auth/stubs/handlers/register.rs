//! Published by `art publish auth`. App code: change it freely.

use auth::Auth;
use route::{Body, Cx, IntoResponse, Json, Redirect, http::{Response, StatusCode}};
use validate::{Errors, Valid};

use crate::{requests::register::Register, state::App, users::{User, Users}};

pub async fn show(_: ()) -> inertia::Page<()> {
    inertia::render("auth/register", ())
}

/// Create the account and log it in. Add your own steps here (welcome mail,
/// default team, ...).
pub async fn store(cx: Cx<App>, (auth, input): (Auth<User>, Valid<Json<Register>>)) -> Result<Redirect, Response<Body>> {
    let users = Users { db: cx.db.clone() };
    let server_error = |e: &dyn std::fmt::Display| {
        tracing::error!("register: {e}");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    };

    if users.by_email(&input.email).await.map_err(|e| server_error(&e))?.is_some() {
        let mut errors = Errors::new();
        errors.add("email", "The email has already been taken.");
        return Err(errors.into_response());
    }
    let hash = auth::hash_password(&input.password).await.map_err(|e| server_error(&e))?;
    let user = users.create(&input.email, &hash).await.map_err(|e| server_error(&e))?;
    auth.login(&user);
    Ok(Redirect::to_route("home"))
}
