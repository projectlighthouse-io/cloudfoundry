//! Published by `art publish auth`. App code: change it freely.
//!
//! Email verification. The link carries a random token (stored hashed) and
//! works for an hour (`auth::VERIFY_EXPIRES`), in any browser.

use auth::{Auth, Authed, VerifyError};
use route::{Body, Cx, IntoResponse, Path, http::{Response, StatusCode}};
use validate::Errors;

use crate::{mailer::deliver, state::App, users::{User, UserStore}};

pub(crate) fn invalid(field: &'static str, message: impl Into<String>) -> Response<Body> {
    let mut errors = Errors::new();
    errors.add(field, message.into());
    errors.into_response()
}

pub(crate) fn failed(e: VerifyError) -> Response<Body> {
    match e {
        VerifyError::AlreadyVerified => invalid("email", "Your email address is already verified."),
        VerifyError::Throttled(wait) => invalid("email", format!("Please wait {} seconds before asking for another link.", wait.as_secs().max(1))),
        VerifyError::EmailTaken => invalid("email", "The email has already been taken."),
        VerifyError::InvalidToken => invalid("token", "This verification link is invalid or has expired."),
        VerifyError::Other(e) => {
            tracing::error!("email verification: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Mail the link to `email`, the address being verified (`resources/mail/verify-email.md`).
/// A mail failure is logged; the user can ask for a new link.
pub(crate) async fn send_verification_link(app: &App, email: &str, token: &str) {
    let link = format!("{}/verify-email/{token}", app.cfg.app_url);
    deliver(app, include_str!("../../resources/mail/verify-email.md"), email, &[("email", email), ("link", &link)]).await;
}

/// Send a new link for the current address: `204`, or `422` (already verified, or too soon).
pub async fn send(cx: Cx<App>, user: Authed<User>) -> Result<StatusCode, Response<Body>> {
    let users = UserStore { db: cx.db.clone() };
    let token = auth::verification_token(&users, &user, None).await.map_err(failed)?;
    send_verification_link(&cx, &user.email, &token).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The link: verifies the address (or switches to the new one): `204`, or `422`.
pub async fn verify(cx: Cx<App>, auth: Auth<User>, Path(token): Path<String>) -> Result<StatusCode, Response<Body>> {
    let users = UserStore { db: cx.db.clone() };
    auth.verify_email(&users, &token).await.map_err(failed)?;
    Ok(StatusCode::NO_CONTENT)
}
