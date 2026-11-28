//! The team a user last switched to: `users.current_team_id`, cached in the
//! session.

use std::{future::Future, ops::Deref, sync::Arc};

use route::{
    Body, FromRequest, FromRequestParts, IntoResponse,
    http::{Request, Response, StatusCode, request::Parts},
};
use session::Session;

use crate::{Error, Membership, Role, inputs::{TeamGate, team_gate}};

const KEY: &str = "current_team";

/// The user's current team. Resolved in order: the session's cached id, the
/// stored one (`users.current_team_id`, so a new login or another app
/// instance lands on the same team), then the first team by id, or a new
/// personal team when [`Teams::require_team`](crate::Teams::require_team)
/// is on. Whatever it falls back to is stored. Each id is checked: a team the
/// user can no longer open is skipped. `None` only without any team.
///
/// Routes still name their team in the path; this only picks where to land
/// and what a switcher shows. Needs the session layer before `teams::layer`.
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
    /// Switch to a team, for this session and the stored default. It takes a
    /// [`Membership`], so only a team the user can open: switch in a handler
    /// whose input loaded it.
    pub async fn set(&mut self, membership: &Membership<R>) -> Result<(), Error> {
        self.gate.0.save_team(self.user, Some(membership.team.id)).await?;
        cache(&self.session, membership.team.id);
        self.team = Some(membership.clone());
        Ok(())
    }

    pub fn into_inner(self) -> Option<Membership<R>> {
        self.team
    }
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

impl<S: Send + Sync, R: Role> FromRequestParts<S> for CurrentTeam<R> {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        let gate = team_gate::<R>(parts)?;
        let session = parts.extensions.get::<Session>().cloned().ok_or_else(|| {
            tracing::error!("CurrentTeam needs `session::layer` before `teams::layer`");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;
        let user = gate.0.user(parts).await?;
        #[allow(clippy::result_large_err, reason = "the error is the response itself, returned once per request")]
        let found = |team: Option<Membership<R>>, session: Session, gate: TeamGate<R>| Ok(Self { team, session, gate, user });

        if let Some(id) = session.get::<i64>(KEY).ok().flatten() {
            if let Some(team) = gate.0.lookup_team(id, user).await.map_err(failed)? {
                return found(Some(team), session, gate);
            }
            session.remove(KEY);
        }
        let saved = gate.0.saved_team(user).await.map_err(failed)?;
        if let Some(id) = saved {
            if let Some(team) = gate.0.lookup_team(id, user).await.map_err(failed)? {
                cache(&session, id);
                return found(Some(team), session, gate);
            }
        }
        let team = gate.0.fallback_team(user).await.map_err(failed)?;
        let id = team.as_ref().map(|m| m.team.id);
        if id != saved {
            gate.0.save_team(user, id).await.map_err(failed)?;
        }
        if let Some(id) = id {
            cache(&session, id);
        }
        found(team, session, gate)
    }
}

impl<S: Send + Sync, R: Role> FromRequest<S> for CurrentTeam<R> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}
