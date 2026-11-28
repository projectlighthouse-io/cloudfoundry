//! Published by `art publish auth --with-ui`. App code: change it freely.
//!
//! Sudo mode: pages whose handlers take `auth::RecentlyConfirmed<User>` send
//! the user here first, then back to where they were going. Accounts without
//! a password (social login) confirm with a passkey (`art publish passkeys`)
//! or by logging in with their provider again; both call `auth.mark_confirmed()`.

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
    /// `false` for social-only accounts: offer a passkey or the provider instead.
    has_password: bool,
}

pub async fn show(user: Authed<User>) -> inertia::Page<ConfirmPasswordPage> {
    inertia::render("auth/confirm-password", ConfirmPasswordPage { email: user.email.clone(), has_password: user.password_hash.is_some() })
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
