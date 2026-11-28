//! Published by `art publish factor2 --with-ui`. App code: change it freely.
//!
//! `login` replaces `handlers::login::login` on `POST /login`: after a correct
//! password it logs in, or (two-factor on, browser not trusted) sends the
//! user to the challenge page.

use auth::{AttemptError, Auth, Authed, RecentlyConfirmed, Provider};
use factor2::{ChallengeError, Code, Device, Setup, Status, TrustedDevice, Uuid, events};
use route::{
    Body, ClientAddr, Cx, IntoResponse, Json, Path, Redirect,
    http::{HeaderValue, Response, StatusCode, header},
};
use serde::Serialize;
use session::Session;
use ts_rs::TS;
use validate::{Errors, Valid};

use crate::{
    requests::{
        login::Login,
        two_factor::{Challenge, ConfirmCode},
    },
    state::App,
    two_factor::two_factor,
    users::{User, UserStore},
};

type Result<T> = std::result::Result<T, Response<Body>>;

const SETTINGS: &str = "/settings/two-factor";
/// New backup codes, flashed for the next page: they are stored hashed, so this is the only time they show.
const CODES: &str = "two_factor.backup_codes";

/// Props for `resources/js/pages/settings/two-factor.vue`.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct TwoFactorPage {
    email: String,
    status: Status,
    /// While confirming: the QR code and secret to add to an authenticator app.
    setup: Option<Setup>,
    /// Right after enabling or regenerating; never again.
    backup_codes: Option<Vec<String>>,
    devices: Vec<TrustedDevice>,
}

fn invalid(field: &'static str, message: impl Into<String>) -> Response<Body> {
    let mut errors = Errors::new();
    errors.add(field, message.into());
    errors.into_response()
}

fn failed(e: factor2::Error) -> Response<Body> {
    match e {
        factor2::Error::InvalidCode => invalid("code", "The provided two factor authentication code was invalid."),
        factor2::Error::AlreadyEnabled => invalid("code", "Two-factor authentication is already enabled."),
        factor2::Error::NotEnabled => invalid("code", "Two-factor authentication is not enabled."),
        factor2::Error::Other(e) => server_error(e),
    }
}

fn server_error(e: impl std::fmt::Display) -> Response<Body> {
    tracing::error!("two-factor: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

/// Listeners are registered in `src/events.rs`; a failing one is logged, not shown.
async fn emit<E: Clone + Send + Sync + 'static>(app: &App, event: E) {
    if let Err(e) = app.events.emit(app, event).await {
        tracing::error!("two-factor event listener failed: {e}");
    }
}

fn with_cookie(redirect: Redirect, cookie: &str) -> Result<Response<Body>> {
    let mut res = redirect.into_response();
    res.headers_mut().append(header::SET_COOKIE, HeaderValue::from_str(cookie).map_err(server_error)?);
    Ok(res)
}

/// The password check of `handlers::login::login`, then the second factor.
pub async fn login(cx: Cx<App>, auth: Auth<User>, session: Session, device: Device, input: Valid<Json<Login>>) -> Result<Redirect> {
    let users = UserStore { db: cx.db.clone() };
    let user = match auth.validate(&users, &input.email, &input.password).await {
        Ok(user) => user,
        Err(AttemptError::Failed) => return Err(invalid("email", "These credentials do not match our records.")),
        Err(AttemptError::Throttled(wait)) => {
            return Err(invalid("email", format!("Too many login attempts. Please try again in {} seconds.", wait.as_secs().max(1))));
        }
        Err(AttemptError::Other(e)) => return Err(server_error(e)),
    };
    let tf = two_factor(&cx);
    if tf.enabled(user.id).await.map_err(failed)? && !tf.trusted(user.id, &device).await.map_err(failed)? {
        tf.begin(&session, user.id, input.remember).map_err(failed)?;
        return Ok(Redirect::to_route("two-factor.login"));
    }
    if input.remember { auth.login_remember(&user) } else { auth.login(&user) }
    Ok(Redirect::to_route("home"))
}

#[derive(Serialize)]
pub struct ChallengePage {}

/// Only after a correct password; otherwise back to the login page.
pub async fn challenge_page(cx: Cx<App>, session: Session) -> std::result::Result<inertia::Page<ChallengePage>, Redirect> {
    match two_factor(&cx).pending(&session) {
        Some(_) => Ok(inertia::render("auth/two-factor-challenge", ChallengePage {})),
        None => Err(Redirect::to_route("login")),
    }
}

/// A code from the app or a backup code finishes the login. With `trust`,
/// this browser skips the challenge for 30 days.
pub async fn challenge(cx: Cx<App>, auth: Auth<User>, session: Session, device: Device, ip: ClientAddr, input: Valid<Json<Challenge>>) -> Result<Response<Body>> {
    let tf = two_factor(&cx);
    let code = match (&input.backup_code, &input.code) {
        (Some(backup), _) => Code::Backup(backup),
        (None, code) => Code::Totp(code.as_deref().unwrap_or_default()),
    };
    let ip = Some(ip.0.ip());
    let passed = match tf.challenge(&session, ip, code).await {
        Ok(passed) => passed,
        Err(ChallengeError::NoPending) => return Ok(Redirect::to_route("login").into_response()),
        Err(ChallengeError::Throttled(wait)) => return Err(invalid("code", format!("Too many attempts. Please try again in {} seconds.", wait.as_secs().max(1)))),
        Err(ChallengeError::InvalidCode { user_id }) => {
            emit(&cx, events::TwoFactorChallengeFailed { user_id, ip }).await;
            let field = if input.backup_code.is_some() { "backup_code" } else { "code" };
            return Err(invalid(field, "The provided two factor authentication code was invalid."));
        }
        Err(ChallengeError::Other(e)) => return Err(failed(e)),
    };
    let users = UserStore { db: cx.db.clone() };
    let Some(user) = users.find(passed.user_id).await.map_err(server_error)? else { return Ok(Redirect::to_route("login").into_response()) };
    if passed.remember { auth.login_remember(&user) } else { auth.login(&user) }
    if let Some(remaining) = passed.backup_codes_left {
        emit(&cx, events::BackupCodeUsed { user_id: user.id, remaining }).await;
    }
    let home = Redirect::to_route("home");
    if input.trust { with_cookie(home, &tf.trust(user.id, &device, ip).await.map_err(failed)?) } else { Ok(home.into_response()) }
}

pub async fn show(cx: Cx<App>, user: Authed<User>, session: Session) -> Result<inertia::Page<TwoFactorPage>> {
    let tf = two_factor(&cx);
    let status = tf.status(user.id).await.map_err(failed)?;
    let setup = if status.confirming { Some(tf.setup(user.id, &user.email).await.map_err(failed)?) } else { None };
    let devices = tf.devices(user.id).await.map_err(failed)?;
    let backup_codes = session.get(CODES).ok().flatten();
    Ok(inertia::render("settings/two-factor", TwoFactorPage { email: user.email.clone(), status, setup, backup_codes, devices }))
}

/// Start enrolment; the page then shows the QR code until `confirm`.
pub async fn enable(cx: Cx<App>, user: Authed<User>) -> Result<Redirect> {
    two_factor(&cx).setup(user.id, &user.email).await.map_err(failed)?;
    Ok(Redirect::to(SETTINGS))
}

pub async fn confirm(cx: Cx<App>, user: Authed<User>, session: Session, input: Valid<Json<ConfirmCode>>) -> Result<Redirect> {
    let codes = two_factor(&cx).confirm(user.id, &input.code).await.map_err(failed)?;
    if let Err(e) = session.flash(CODES, &codes) {
        tracing::error!("flashing backup codes: {e}");
    }
    emit(&cx, events::TwoFactorEnabled { user_id: user.id }).await;
    Ok(Redirect::to(SETTINGS))
}

/// Needs a recent password confirmation: the user is sent to confirm it first.
pub async fn disable(cx: Cx<App>, user: RecentlyConfirmed<User>) -> Result<Response<Body>> {
    let tf = two_factor(&cx);
    tf.disable(user.id).await.map_err(failed)?;
    emit(&cx, events::TwoFactorDisabled { user_id: user.id }).await;
    with_cookie(Redirect::to(SETTINGS), &tf.forget_cookie())
}

/// New backup codes; the old ones stop working. Needs a recent password confirmation.
pub async fn regenerate(cx: Cx<App>, user: RecentlyConfirmed<User>, session: Session) -> Result<Redirect> {
    let codes = two_factor(&cx).regenerate_backup_codes(user.id).await.map_err(failed)?;
    if let Err(e) = session.flash(CODES, &codes) {
        tracing::error!("flashing backup codes: {e}");
    }
    Ok(Redirect::to(SETTINGS))
}

pub async fn revoke(cx: Cx<App>, user: Authed<User>, Path(device): Path<Uuid<TrustedDevice>>) -> Result<Redirect> {
    match two_factor(&cx).revoke(user.id, device).await.map_err(failed)? {
        true => Ok(Redirect::to(SETTINGS)),
        false => Err(StatusCode::NOT_FOUND.into_response()),
    }
}
