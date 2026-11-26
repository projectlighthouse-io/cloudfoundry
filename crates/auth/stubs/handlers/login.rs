//! Published by `art publish auth`. App code: change it freely.

use auth::Auth;
use route::{Body, Cx, IntoResponse, Json, Redirect, http::Response};
use validate::{Errors, Valid};

use crate::{error::AppError, requests::login::Login, state::App, users::{User, Users}};

pub async fn login(cx: Cx<App>, (auth, input): (Auth<User>, Valid<Json<Login>>)) -> Result<Redirect, Response<Body>> {
    let failed = || {
        let mut e = Errors::new();
        e.add("email", "These credentials do not match our records.");
        e.into_response()
    };

    let users = Users { db: cx.db.clone() };
    let user = users.by_email(&input.email).await.map_err(|e| AppError::from(e).into_response())?;
    let Some(user) = user else {
        password::verify_none(&input.password);
        return Err(failed());
    };
    password::verify(&input.password, &user.password_hash).map_err(|_| failed())?;

    auth.login(&user);
    Ok(Redirect::to_route("home"))
}

pub async fn logout(auth: Auth<User>) -> Redirect {
    auth.logout();
    Redirect::to_route("home")
}
