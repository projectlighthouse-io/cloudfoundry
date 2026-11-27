//! The team a user last switched to, kept in the session.

use std::{future::Future, ops::Deref, sync::Arc};

use route::{
    Body, FromRequest, FromRequestParts, IntoResponse,
    http::{Request, Response, StatusCode, request::Parts},
};
use session::Session;

use crate::{Membership, Role, inputs::team_gate};

const KEY: &str = "current_team";

/// The user's current team: the one they last switched to, if they can
/// still open it, else their first team by id. `None` without any team.
/// Routes still name their team in the path; this only picks where to land
/// and what a switcher shows. Needs the session layer before `teams::layer`.
pub struct CurrentTeam<R> {
    team: Option<Membership<R>>,
    session: Session,
}

impl<R> Deref for CurrentTeam<R> {
    type Target = Option<Membership<R>>;
    fn deref(&self) -> &Option<Membership<R>> {
        &self.team
    }
}

impl<R: Clone> CurrentTeam<R> {
    /// Switch to a team. It takes a [`Membership`], so only a team the user
    /// can open: switch in a handler whose input loaded it.
    pub fn set(&mut self, membership: &Membership<R>) {
        if let Err(e) = self.session.insert(KEY, &membership.team.id) {
            tracing::error!("teams: storing the current team failed: {e}");
        }
        self.team = Some(membership.clone());
    }

    /// Forget the choice; the next request falls back to the first team.
    pub fn clear(&mut self) {
        self.session.remove(KEY);
        self.team = None;
    }

    pub fn into_inner(self) -> Option<Membership<R>> {
        self.team
    }
}

fn failed(e: crate::Error) -> Response<Body> {
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
        if let Some(id) = session.get::<i64>(KEY).ok().flatten() {
            if let Some(team) = gate.0.lookup_team(id, user).await.map_err(failed)? {
                return Ok(Self { team: Some(team), session });
            }
            session.remove(KEY);
        }
        let team = gate.0.first_team(user).await.map_err(failed)?;
        Ok(Self { team, session })
    }
}

impl<S: Send + Sync, R: Role> FromRequest<S> for CurrentTeam<R> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}
