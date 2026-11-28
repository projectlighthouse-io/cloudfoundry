//! The team a user picked: `users.current_team_id`, cached in the session,
//! and [`require_current_team`] for routes that need one.

use std::{
    future::Future,
    marker::PhantomData,
    ops::Deref,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use route::{
    Body, FromRequest, FromRequestParts, IntoResponse, Json, Redirect,
    http::{Method, Request, Response, StatusCode, header, request::Parts},
};
use serde::Serialize;
use session::Session;
use tower::{Layer, Service};

use crate::{Error, Membership, Role, inputs::{TeamGate, team_gate}};

const KEY: &str = "current_team";
/// The page [`require_current_team`] sent the user away from.
const INTENDED: &str = "_teams_intended";

/// The team the user picked. Resolved from the session's cached id, then the
/// stored one (`users.current_team_id`, so a new login lands on the same
/// team). There is no fallback: a user picks a team explicitly, with
/// [`CurrentTeam::set`]. A saved team the user can no longer open (deleted,
/// or they were removed) is cleared, and this is `None`.
///
/// Routes behind [`require_current_team`] always see `Some`. Needs the
/// session layer before `teams::layer`.
pub struct CurrentTeam<R> {
    team: Option<Membership<R>>,
    session: Session,
    gate: TeamGate<R>,
    user: i64,
}

impl<R> Deref for CurrentTeam<R> {
    type Target = Option<Membership<R>>;
    fn deref(&self) -> &Option<Membership<R>> {
        &self.team
    }
}

impl<R: Clone> CurrentTeam<R> {
    /// Pick a team, for this session and the stored default. It takes a
    /// [`Membership`], so only a team the user can open: pick in a handler
    /// whose input loaded it.
    pub async fn set(&mut self, membership: &Membership<R>) -> Result<(), Error> {
        self.gate.0.save_team(self.user, Some(membership.team.id)).await?;
        cache(&self.session, membership.team.id);
        self.team = Some(membership.clone());
        Ok(())
    }

    /// Forget the pick: routes behind [`require_current_team`] ask again.
    pub async fn clear(&mut self) -> Result<(), Error> {
        self.gate.0.save_team(self.user, None).await?;
        self.session.remove(KEY);
        self.team = None;
        Ok(())
    }

    pub fn into_inner(self) -> Option<Membership<R>> {
        self.team
    }
}

/// Where [`require_current_team`] was going before it sent the user to pick a
/// team, or `default`. Taken: redirect there after [`CurrentTeam::set`].
pub fn intended(session: &Session, default: &str) -> String {
    let url = session.get::<String>(INTENDED).ok().flatten();
    session.remove(INTENDED);
    url.unwrap_or_else(|| default.to_owned())
}

fn cache(session: &Session, team: i64) {
    if let Err(e) = session.insert(KEY, &team) {
        tracing::error!("teams: caching the current team failed: {e}");
    }
}

fn failed(e: Error) -> Response<Body> {
    tracing::error!("teams: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

/// The team [`require_current_team`] resolved, so the input does not look again.
struct Resolved<R>(Membership<R>);

impl<R: Clone> Clone for Resolved<R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[allow(clippy::result_large_err, reason = "the error is the response itself, returned once per request")]
fn session_of(parts: &Parts) -> Result<Session, Response<Body>> {
    parts.extensions.get::<Session>().cloned().ok_or_else(|| {
        tracing::error!("CurrentTeam needs `session::layer` before `teams::layer`");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
}

async fn resolve<R: Role>(parts: &mut Parts) -> Result<CurrentTeam<R>, Response<Body>> {
    let gate = team_gate::<R>(parts)?;
    let session = session_of(parts)?;
    let user = gate.0.user(parts).await?;
    if let Some(Resolved(m)) = parts.extensions.get::<Resolved<R>>().cloned() {
        return Ok(CurrentTeam { team: Some(m), session, gate, user });
    }
    let saved = gate.0.saved_team(user).await.map_err(failed)?;
    let cached = session.get::<i64>(KEY).ok().flatten();
    let mut team = None;
    for id in [cached, saved].into_iter().flatten() {
        team = gate.0.lookup_team(id, user).await.map_err(failed)?;
        if team.is_some() {
            break;
        }
    }
    if let Some(m) = &team {
        if cached != Some(m.team.id) {
            cache(&session, m.team.id);
        }
    } else {
        // Deleted, or the user was removed: forget it, so they pick again.
        if cached.is_some() {
            session.remove(KEY);
        }
        if saved.is_some() {
            gate.0.save_team(user, None).await.map_err(failed)?;
        }
    }
    Ok(CurrentTeam { team, session, gate, user })
}

impl<S: Send + Sync, R: Role> FromRequestParts<S> for CurrentTeam<R> {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        resolve(parts).await
    }
}

impl<S: Send + Sync, R: Role> FromRequest<S> for CurrentTeam<R> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}

/// Only requests with a current team pass: [`CurrentTeam<R>`] is then `Some`.
/// Otherwise page visits go to the team picker (`/teams/select`), or to
/// "create a team" (`/teams/create`) for a user without any team, and the
/// page they wanted is kept for [`intended`]. JSON clients get `409` with
/// `{"error": "team_required"}`. Anonymous requests get what the user lookup
/// answers (with `teams::auth_user`: the login redirect or `401`).
///
/// Leave the picker, "create a team", invitations, account settings and
/// logout outside the group this guards.
pub fn require_current_team<R>() -> RequireCurrentTeam<R> {
    RequireCurrentTeam { select: "/teams/select".into(), create: "/teams/create".into(), role: PhantomData }
}

pub struct RequireCurrentTeam<R> {
    select: Arc<str>,
    create: Arc<str>,
    role: PhantomData<fn() -> R>,
}

impl<R> RequireCurrentTeam<R> {
    /// The team picker. Default `/teams/select`.
    pub fn select_path(mut self, path: &str) -> Self {
        self.select = path.into();
        self
    }

    /// Where a user without any team goes. Default `/teams/create`.
    pub fn create_path(mut self, path: &str) -> Self {
        self.create = path.into();
        self
    }
}

impl<R> Clone for RequireCurrentTeam<R> {
    fn clone(&self) -> Self {
        Self { select: self.select.clone(), create: self.create.clone(), role: PhantomData }
    }
}

impl<S, R> Layer<S> for RequireCurrentTeam<R> {
    type Service = RequireCurrentTeamService<S, R>;

    fn layer(&self, inner: S) -> Self::Service {
        RequireCurrentTeamService { inner, guard: self.clone() }
    }
}

pub struct RequireCurrentTeamService<S, R> {
    inner: S,
    guard: RequireCurrentTeam<R>,
}

impl<S: Clone, R> Clone for RequireCurrentTeamService<S, R> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), guard: self.guard.clone() }
    }
}

#[derive(Serialize)]
struct TeamRequired {
    error: &'static str,
}

/// JSON clients get the 409; page visits (Inertia ones too) the redirect.
fn wants_json(parts: &Parts) -> bool {
    !parts.headers.contains_key("x-inertia")
        && parts.headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("application/json"))
}

impl<S, R, ReqB> Service<Request<ReqB>> for RequireCurrentTeamService<S, R>
where
    S: Service<Request<ReqB>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send,
    R: Role,
    ReqB: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqB>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let guard = self.guard.clone();
        Box::pin(async move {
            let (mut parts, body) = req.into_parts();
            let current = match resolve::<R>(&mut parts).await {
                Ok(c) => c,
                Err(res) => return Ok(res),
            };
            if let Some(m) = current.team {
                parts.extensions.insert(Resolved(m));
                return inner.call(Request::from_parts(parts, body)).await;
            }
            if wants_json(&parts) {
                return Ok((StatusCode::CONFLICT, Json(TeamRequired { error: "team_required" })).into_response());
            }
            if parts.method == Method::GET {
                let path = parts.uri.path_and_query().map_or("/", |p| p.as_str()).to_owned();
                if let Err(e) = current.session.insert(INTENDED, &path) {
                    tracing::error!("teams: remembering the intended page failed: {e}");
                }
            }
            let has_team = match current.gate.0.has_team(current.user).await {
                Ok(has) => has,
                Err(e) => return Ok(failed(e)),
            };
            Ok(Redirect::to(if has_team { guard.select.to_string() } else { guard.create.to_string() }).into_response())
        })
    }
}
