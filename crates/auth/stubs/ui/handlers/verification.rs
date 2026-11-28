//! Published by `art publish auth --with-ui`. App code: change it freely.
//!
//! Email verification. The link carries a random token (stored hashed) and
//! works for an hour (`auth::VERIFY_EXPIRES`), in any browser. Routes that
//! take `auth::Verified<User>` send unverified users to `notice`.

use auth::{Auth, Authed, VerifyError};
use route::{Body, Cx, IntoResponse, Path, Redirect, http::{Response, StatusCode}};
use serde::Serialize;
use session::Session;
use ts_rs::TS;
use validate::Errors;

use crate::{mailer::mailer, state::App, users::{User, UserStore}};

/// Shown once on the next page.
pub(crate) const STATUS: &str = "status";

/// Props for `resources/js/pages/auth/verify-email.vue`.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct VerifyEmailPage {
    email: String,
    status: Option<String>,
}

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

/// Mail the link to `email`, the address being verified. A mail failure is
/// logged; the user can ask for a new link.
pub(crate) async fn send_verification_link(app: &App, email: &str, token: &str) {
    let link = format!("{}/verify-email/{token}", app.cfg.app_url);
    let text = format!("Confirm that {email} is your address (the link works for an hour):
{link}

If you did not ask for this, ignore this mail.");
    if let Err(e) = mailer(app).send(mail::Message::new(email, "Verify your email address", text)).await {
        tracing::error!("verification mail to {email}: {e}");
    }
}

/// "Check your inbox", with a button for a new link. Verified users go home.
pub async fn notice(user: Authed<User>, session: Session) -> std::result::Result<inertia::Page<VerifyEmailPage>, Redirect> {
    if user.email_verified_at.is_some() {
        return Err(Redirect::to_route("home"));
    }
    let status = session.get(STATUS).ok().flatten();
    Ok(inertia::render("auth/verify-email", VerifyEmailPage { email: user.email.clone(), status }))
}

/// A new link for the current address; at most one a minute.
pub async fn send(cx: Cx<App>, user: Authed<User>, session: Session) -> Result<Redirect, Response<Body>> {
    let users = UserStore { db: cx.db.clone() };
    let token = auth::verification_token(&users, &user, None).await.map_err(failed)?;
    send_verification_link(&cx, &user.email, &token).await;
    if let Err(e) = session.flash(STATUS, "A new verification link is on its way.") {
        tracing::error!("flash: {e}");
    }
    Ok(Redirect::to_route("verification.notice"))
}

/// The link: verifies the address (or switches to the new one), then home.
/// A bad or old link shows the notice page with the reason.
pub async fn verify(cx: Cx<App>, auth: Auth<User>, session: Session, Path(token): Path<String>) -> Result<Redirect, Response<Body>> {
    let users = UserStore { db: cx.db.clone() };
    let message = match auth.verify_email(&users, &token).await {
        Ok(_) => "Your email address is verified.",
        Err(VerifyError::InvalidToken) => "This verification link is invalid or has expired. Ask for a new one.",
        Err(VerifyError::EmailTaken) => "That email address is now used by another account.",
        Err(e) => return Err(failed(e)),
    };
    if let Err(e) = session.flash(STATUS, message) {
        tracing::error!("flash: {e}");
    }
    Ok(match auth.user() {
        Some(_) => Redirect::to_route("email.edit"),
        None => Redirect::to_route("login"),
    })
}
