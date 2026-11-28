//! Published by `art publish passkeys --with-ui`. App code: change it freely.
//!
//! Each ceremony is two requests: `*_options` answers JSON options for
//! `navigator.credentials.create/get` (keeping the challenge in the session),
//! then `resources/js/passkeys.ts` posts `credential.toJSON()` back through
//! Inertia. `second_factor*` need `art publish factor2`; without it delete them and the `factor2` and `two_factor` imports.

use auth::{Auth, Authed, RecentlyConfirmed, Provider};
use factor2::Device;
use passkeys::{CreationChallengeResponse, Passkey, RequestChallengeResponse, Uuid};
use route::{
    Body, ClientAddr, Cx, IntoResponse, Json, Path, Redirect,
    http::{HeaderValue, Response, StatusCode, header},
};
use serde::Serialize;
use session::Session;
use ts_rs::TS;
use validate::{Errors, Valid};

use crate::{
    passkey::passkeys,
    requests::passkey::{AddPasskey, RenamePasskey, UsePasskey},
    state::App,
    two_factor::two_factor,
    users::{User, UserStore},
};

type Result<T> = std::result::Result<T, Response<Body>>;

const SETTINGS: &str = "/settings/passkeys";

/// Props for `resources/js/pages/settings/passkeys.vue`.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct PasskeysPage {
    email: String,
    passkeys: Vec<Passkey>,
}

fn invalid(message: impl Into<String>) -> Response<Body> {
    let mut errors = Errors::new();
    errors.add("passkey", message.into());
    errors.into_response()
}

fn server_error(e: impl std::fmt::Display) -> Response<Body> {
    tracing::error!("passkeys: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

fn failed(e: passkeys::Error) -> Response<Body> {
    match e {
        passkeys::Error::NoCeremony => invalid("The passkey request expired. Please try again."),
        passkeys::Error::NoPasskeys => invalid("You have no passkeys."),
        passkeys::Error::Rejected(e) => {
            tracing::debug!("passkey rejected: {e}");
            invalid("The passkey could not be verified.")
        }
        passkeys::Error::AlreadyRegistered => invalid("This passkey is already registered."),
        passkeys::Error::UnknownCredential => invalid("This passkey is not registered."),
        passkeys::Error::Throttled(wait) => invalid(format!("Too many attempts. Please try again in {} seconds.", wait.as_secs().max(1))),
        passkeys::Error::Other(e) => server_error(e),
    }
}

async fn log_in(cx: &App, auth: &Auth<User>, user_id: i64, remember: bool) -> Result<Redirect> {
    let users = UserStore { db: cx.db.clone() };
    let Some(user) = users.find(user_id).await.map_err(server_error)? else { return Err(invalid("This passkey is not registered.")) };
    if remember { auth.login_remember(&user) } else { auth.login(&user) }
    Ok(Redirect::to_route("home"))
}

pub async fn index(cx: Cx<App>, user: Authed<User>) -> Result<inertia::Page<PasskeysPage>> {
    let passkeys = passkeys(&cx).passkeys(user.id).await.map_err(failed)?;
    Ok(inertia::render("settings/passkeys", PasskeysPage { email: user.email.clone(), passkeys }))
}

pub async fn register_options(cx: Cx<App>, user: Authed<User>, session: Session) -> Result<Json<CreationChallengeResponse>> {
    Ok(Json(passkeys(&cx).start_registration(&session, user.id, &user.email, &user.email).await.map_err(failed)?))
}

pub async fn store(cx: Cx<App>, user: Authed<User>, session: Session, input: Valid<Json<AddPasskey>>) -> Result<Redirect> {
    passkeys(&cx).finish_registration(&session, user.id, &input.name, &input.credential).await.map_err(failed)?;
    Ok(Redirect::to(SETTINGS))
}

pub async fn rename(cx: Cx<App>, user: Authed<User>, Path(passkey): Path<Uuid<Passkey>>, input: Valid<Json<RenamePasskey>>) -> Result<Redirect> {
    match passkeys(&cx).rename(user.id, passkey, &input.name).await.map_err(failed)? {
        true => Ok(Redirect::to(SETTINGS)),
        false => Err(StatusCode::NOT_FOUND.into_response()),
    }
}

/// Needs a recent password confirmation: the user is sent to confirm it first.
pub async fn destroy(cx: Cx<App>, user: RecentlyConfirmed<User>, Path(passkey): Path<Uuid<Passkey>>) -> Result<Redirect> {
    match passkeys(&cx).remove(user.id, passkey).await.map_err(failed)? {
        true => Ok(Redirect::to(SETTINGS)),
        false => Err(StatusCode::NOT_FOUND.into_response()),
    }
}

/// Passwordless: no username, the browser offers the passkeys it holds for this site.
pub async fn login_options(cx: Cx<App>, session: Session) -> Result<Json<RequestChallengeResponse>> {
    Ok(Json(passkeys(&cx).start_login(&session).map_err(failed)?))
}

/// A passkey verifies the user itself (PIN or biometrics): no two-factor challenge.
pub async fn login(cx: Cx<App>, auth: Auth<User>, session: Session, ip: ClientAddr, input: Valid<Json<UsePasskey>>) -> Result<Redirect> {
    let used = passkeys(&cx).finish_login(&session, Some(ip.0.ip()), &input.credential).await.map_err(failed)?;
    log_in(&cx, &auth, used.user_id, false).await
}

/// On the two-factor challenge page, instead of a code: only after a correct password.
pub async fn second_factor_options(cx: Cx<App>, session: Session) -> Result<Json<RequestChallengeResponse>> {
    let Some(user) = two_factor(&cx).pending(&session) else { return Err(StatusCode::CONFLICT.into_response()) };
    Ok(Json(passkeys(&cx).start_authentication(&session, user).await.map_err(failed)?))
}

/// With `trust`, this browser skips the two-factor challenge for 30 days, as after a code.
pub async fn second_factor(cx: Cx<App>, auth: Auth<User>, session: Session, device: Device, ip: ClientAddr, input: Valid<Json<UsePasskey>>) -> Result<Response<Body>> {
    let tf = two_factor(&cx);
    let Some(user) = tf.pending(&session) else { return Ok(Redirect::to_route("login").into_response()) };
    let ip = Some(ip.0.ip());
    passkeys(&cx).finish_authentication(&session, ip, user, &input.credential).await.map_err(failed)?;
    let Ok(passed) = tf.pass(&session) else { return Ok(Redirect::to_route("login").into_response()) };
    let mut res = log_in(&cx, &auth, passed.user_id, passed.remember).await?.into_response();
    if input.trust {
        let cookie = tf.trust(passed.user_id, &device, ip).await.map_err(server_error)?;
        res.headers_mut().append(header::SET_COOKIE, HeaderValue::from_str(&cookie).map_err(server_error)?);
    }
    Ok(res)
}

/// Sudo mode with a passkey: for accounts without a password, or instead of
/// typing it. Then `auth::RecentlyConfirmed<User>` passes.
pub async fn confirm_options(cx: Cx<App>, user: Authed<User>, session: Session) -> Result<Json<RequestChallengeResponse>> {
    Ok(Json(passkeys(&cx).start_authentication(&session, user.id).await.map_err(failed)?))
}

pub async fn confirm(cx: Cx<App>, auth: Auth<User>, user: Authed<User>, session: Session, ip: ClientAddr, input: Valid<Json<UsePasskey>>) -> Result<Redirect> {
    passkeys(&cx).finish_authentication(&session, Some(ip.0.ip()), user.id, &input.credential).await.map_err(failed)?;
    auth.mark_confirmed().map_err(server_error)?;
    Ok(Redirect::to(auth.intended("/")))
}
