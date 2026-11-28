//! Published by `art publish factor2`. App code: change it freely.
//!
//! `login` replaces `handlers::login::login` on `POST /login`: after a correct
//! password it logs in, or (two-factor on, browser not trusted) answers
//! `{ "two_factor": true }` and waits for `POST /two-factor-challenge`.

use auth::{AttemptError, Auth, Authed, RecentlyConfirmed, Provider};
use factor2::{ChallengeError, Code, Device, Setup, Status, TrustedDevice, Uuid, events};
use route::{
    Body, ClientAddr, Cx, IntoResponse, Json, Path,
    http::{HeaderValue, Response, StatusCode, header},
};
use serde::Serialize;
use session::Session;
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

#[derive(Serialize)]
pub struct LoginResult {
    /// `true`: send a code to `POST /two-factor-challenge` to finish logging in.
    two_factor: bool,
}

#[derive(Serialize)]
pub struct TwoFactorSettings {
    status: Status,
    devices: Vec<TrustedDevice>,
}

/// Shown once: only hashes are stored.
#[derive(Serialize)]
pub struct BackupCodes {
    backup_codes: Vec<String>,
}

pub(crate) fn invalid(field: &'static str, message: impl Into<String>) -> Response<Body> {
    let mut errors = Errors::new();
    errors.add(field, message.into());
    errors.into_response()
}

pub(crate) fn failed(e: factor2::Error) -> Response<Body> {
    match e {
        factor2::Error::InvalidCode => invalid("code", "The provided two factor authentication code was invalid."),
        factor2::Error::AlreadyEnabled => invalid("code", "Two-factor authentication is already enabled."),
        factor2::Error::NotEnabled => invalid("code", "Two-factor authentication is not enabled."),
        factor2::Error::Other(e) => {
            tracing::error!("two-factor: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Listeners are registered in `src/events.rs`; a failing one is logged, not shown.
pub(crate) async fn emit<E: Clone + Send + Sync + 'static>(app: &App, event: E) {
    if let Err(e) = app.events.emit(app, event).await {
        tracing::error!("two-factor event listener failed: {e}");
    }
}

fn server_error(e: impl std::fmt::Display) -> Response<Body> {
    tracing::error!("two-factor: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

/// The password check of `handlers::login::login`, then the second factor.
pub async fn login(cx: Cx<App>, auth: Auth<User>, session: Session, device: Device, input: Valid<Json<Login>>) -> Result<Json<LoginResult>> {
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
        return Ok(Json(LoginResult { two_factor: true }));
    }
    if input.remember { auth.login_remember(&user) } else { auth.login(&user) }
    Ok(Json(LoginResult { two_factor: false }))
}

/// A code from the app or a backup code finishes the login (`204`). With
/// `trust`, this browser skips the challenge for 30 days.
pub async fn challenge(cx: Cx<App>, auth: Auth<User>, session: Session, device: Device, ip: ClientAddr, input: Valid<Json<Challenge>>) -> Result<Response<Body>> {
    let tf = two_factor(&cx);
    let code = match (&input.backup_code, &input.code) {
        (Some(backup), _) => Code::Backup(backup),
        (None, code) => Code::Totp(code.as_deref().unwrap_or_default()),
    };
    let ip = Some(ip.0.ip());
    let passed = match tf.challenge(&session, ip, code).await {
        Ok(passed) => passed,
        Err(ChallengeError::NoPending) => return Err(invalid("code", "Your login has expired. Log in again.")),
        Err(ChallengeError::Throttled(wait)) => return Err(invalid("code", format!("Too many attempts. Please try again in {} seconds.", wait.as_secs().max(1)))),
        Err(ChallengeError::InvalidCode { user_id }) => {
            emit(&cx, events::TwoFactorChallengeFailed { user_id, ip }).await;
            let field = if input.backup_code.is_some() { "backup_code" } else { "code" };
            return Err(invalid(field, "The provided two factor authentication code was invalid."));
        }
        Err(ChallengeError::Other(e)) => return Err(failed(e)),
    };
    let users = UserStore { db: cx.db.clone() };
    let user = users.find(passed.user_id).await.map_err(server_error)?.ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;
    if passed.remember { auth.login_remember(&user) } else { auth.login(&user) }
    if let Some(remaining) = passed.backup_codes_left {
        emit(&cx, events::BackupCodeUsed { user_id: user.id, remaining }).await;
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    if input.trust {
        let cookie = tf.trust(user.id, &device, ip).await.map_err(failed)?;
        res.headers_mut().append(header::SET_COOKIE, HeaderValue::from_str(&cookie).map_err(server_error)?);
    }
    Ok(res)
}

pub async fn show(cx: Cx<App>, user: Authed<User>) -> Result<Json<TwoFactorSettings>> {
    let tf = two_factor(&cx);
    let status = tf.status(user.id).await.map_err(failed)?;
    let devices = tf.devices(user.id).await.map_err(failed)?;
    Ok(Json(TwoFactorSettings { status, devices }))
}

/// Start enrolment: the secret, its `otpauth://` URI and a QR code. Not on
/// until `confirm` gets a first valid code.
pub async fn enable(cx: Cx<App>, user: Authed<User>) -> Result<Json<Setup>> {
    two_factor(&cx).setup(user.id, &user.email).await.map(Json).map_err(failed)
}

/// Turn two-factor on with a first code; the backup codes are in the response, once.
pub async fn confirm(cx: Cx<App>, user: Authed<User>, input: Valid<Json<ConfirmCode>>) -> Result<Json<BackupCodes>> {
    let backup_codes = two_factor(&cx).confirm(user.id, &input.code).await.map_err(failed)?;
    emit(&cx, events::TwoFactorEnabled { user_id: user.id }).await;
    Ok(Json(BackupCodes { backup_codes }))
}

/// Needs a recent password confirmation (`423` otherwise).
pub async fn disable(cx: Cx<App>, user: RecentlyConfirmed<User>) -> Result<Response<Body>> {
    let tf = two_factor(&cx);
    tf.disable(user.id).await.map_err(failed)?;
    emit(&cx, events::TwoFactorDisabled { user_id: user.id }).await;
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut().append(header::SET_COOKIE, HeaderValue::from_str(&tf.forget_cookie()).map_err(server_error)?);
    Ok(res)
}

/// New backup codes; the old ones stop working. Needs a recent password confirmation.
pub async fn regenerate(cx: Cx<App>, user: RecentlyConfirmed<User>) -> Result<Json<BackupCodes>> {
    let backup_codes = two_factor(&cx).regenerate_backup_codes(user.id).await.map_err(failed)?;
    Ok(Json(BackupCodes { backup_codes }))
}

/// Stop trusting one of the user's browsers. `404` if it is not theirs.
pub async fn revoke(cx: Cx<App>, user: Authed<User>, Path(device): Path<Uuid<TrustedDevice>>) -> Result<StatusCode> {
    match two_factor(&cx).revoke(user.id, device).await.map_err(failed)? {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(StatusCode::NOT_FOUND.into_response()),
    }
}
