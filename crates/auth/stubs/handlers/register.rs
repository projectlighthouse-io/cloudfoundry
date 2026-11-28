//! Published by `art publish auth`. App code: change it freely.

use auth::Auth;
use route::{Body, Cx, IntoResponse, Json, http::{Response, StatusCode}};
use validate::{Errors, Valid};

use crate::{requests::register::Register, state::App, users::{User, UserStore}};

/// Create the account, send the verification link and log it in: `204`, or `422` with field errors.
pub async fn store(cx: Cx<App>, auth: Auth<User>, input: Valid<Json<Register>>) -> Result<StatusCode, Response<Body>> {
    let users = UserStore { db: cx.db.clone() };
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
    let user = users.create(input.name.as_deref().map(str::trim).filter(|n| !n.is_empty()), &input.email, &hash).await.map_err(|e| server_error(&e))?;
    match auth::verification_token(&users, &user, None).await {
        Ok(token) => crate::handlers::verification::send_verification_link(&cx, &user.email, &token).await,
        Err(e) => tracing::error!("register: verification link: {e}"),
    }
    auth.login(&user);
    Ok(StatusCode::NO_CONTENT)
}
