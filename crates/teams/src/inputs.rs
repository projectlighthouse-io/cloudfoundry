//! The layer and the handler inputs that check access.

use std::{
    future::Future,
    marker::PhantomData,
    ops::Deref,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use route::{
    Body, FromRequest, FromRequestParts, IntoResponse, Path,
    http::{Request, Response, StatusCode, request::Parts},
};
use serde::Deserialize;
use tower::{Layer, Service};

use crate::{Error, Grants, Org, OrgRole, Permission, Uuid, Role, Store, Team, Teams};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Who the current user is. [`auth_user`] reads the `auth` crate's user;
/// a closure `Fn(&Parts) -> Option<i64>` works too, e.g. reading your own
/// request extension. No user is `401`, or whatever `auth` answers.
pub trait Identify: Send + Sync + 'static {
    fn user_id<'a>(&'a self, parts: &'a mut Parts) -> BoxFuture<'a, Result<i64, Response<Body>>>;
}

impl<F: Fn(&Parts) -> Option<i64> + Send + Sync + 'static> Identify for F {
    fn user_id<'a>(&'a self, parts: &'a mut Parts) -> BoxFuture<'a, Result<i64, Response<Body>>> {
        let id = self(parts).ok_or_else(|| StatusCode::UNAUTHORIZED.into_response());
        Box::pin(std::future::ready(id))
    }
}

/// The id of `auth`'s logged-in user: `teams::auth_user(|u: &User| u.id)`.
/// Anonymous requests get `auth`'s answer (a redirect to login, or `401`).
#[cfg(feature = "auth")]
pub fn auth_user<U, F>(id: F) -> AuthUser<U, F>
where
    U: Clone + Send + Sync + 'static,
    F: Fn(&U) -> i64 + Send + Sync + 'static,
{
    AuthUser(id, PhantomData)
}

#[cfg(feature = "auth")]
pub struct AuthUser<U, F>(F, PhantomData<fn() -> U>);

#[cfg(feature = "auth")]
impl<U, F> Identify for AuthUser<U, F>
where
    U: Clone + Send + Sync + 'static,
    F: Fn(&U) -> i64 + Send + Sync + 'static,
{
    fn user_id<'a>(&'a self, parts: &'a mut Parts) -> BoxFuture<'a, Result<i64, Response<Body>>> {
        Box::pin(async move {
            let state = Arc::new(());
            let user = auth::Authed::<U>::from_request_parts(parts, &state).await?;
            Ok((self.0)(&user.0))
        })
    }
}

/// Lookups the inputs need, without naming the store type.
trait Lookup<R, O>: Send + Sync {
    fn team(&self, team: i64, user: i64) -> BoxFuture<'_, Result<Option<Membership<R>>, Error>>;
    fn team_public(&self, team: Uuid<Team>, user: i64) -> BoxFuture<'_, Result<Option<Membership<R>>, Error>>;
    fn org_public(&self, org: Uuid<Org>, user: i64) -> BoxFuture<'_, Result<Option<OrgMembership<O>>, Error>>;
    #[cfg_attr(not(feature = "session"), allow(dead_code))]
    fn fallback_team(&self, user: i64) -> BoxFuture<'_, Result<Option<Membership<R>>, Error>>;
    #[cfg_attr(not(feature = "session"), allow(dead_code))]
    fn saved_team(&self, user: i64) -> BoxFuture<'_, Result<Option<i64>, Error>>;
    #[cfg_attr(not(feature = "session"), allow(dead_code))]
    fn save_team(&self, user: i64, team: Option<i64>) -> BoxFuture<'_, Result<(), Error>>;
}

impl<S: Store> Lookup<S::Role, S::OrgRole> for Teams<S> {
    fn team(&self, team: i64, user: i64) -> BoxFuture<'_, Result<Option<Membership<S::Role>>, Error>> {
        Box::pin(self.team_access(team, user))
    }
    fn team_public(&self, team: Uuid<Team>, user: i64) -> BoxFuture<'_, Result<Option<Membership<S::Role>>, Error>> {
        Box::pin(self.team_access_by_uuid(team, user))
    }
    fn org_public(&self, org: Uuid<Org>, user: i64) -> BoxFuture<'_, Result<Option<OrgMembership<S::OrgRole>>, Error>> {
        Box::pin(self.org_access_by_uuid(org, user))
    }
    fn fallback_team(&self, user: i64) -> BoxFuture<'_, Result<Option<Membership<S::Role>>, Error>> {
        Box::pin(Teams::fallback_team(self, user))
    }
    fn saved_team(&self, user: i64) -> BoxFuture<'_, Result<Option<i64>, Error>> {
        Box::pin(self.store().current_team_id(user))
    }
    fn save_team(&self, user: i64, team: Option<i64>) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(self.store().set_current_team_id(user, team))
    }
}

/// In request extensions; the inputs find it by role type.
pub(crate) struct Gate<R, O> {
    lookup: Arc<dyn Lookup<R, O>>,
    identify: Arc<dyn Identify>,
}

impl<R, O> Clone for Gate<R, O> {
    fn clone(&self) -> Self {
        Self { lookup: self.lookup.clone(), identify: self.identify.clone() }
    }
}

/// The team role's gate, by team role type only (what `Membership<R>` knows).
pub(crate) struct TeamGate<R>(pub(crate) Arc<dyn TeamOnly<R>>);
/// The organization role's gate, by organization role type only.
pub(crate) struct OrgGate<O>(Arc<dyn OrgOnly<O>>);

impl<R> Clone for TeamGate<R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<O> Clone for OrgGate<O> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub(crate) trait TeamOnly<R>: Send + Sync {
    fn user<'a>(&'a self, parts: &'a mut Parts) -> BoxFuture<'a, Result<i64, Response<Body>>>;
    #[cfg_attr(not(feature = "session"), allow(dead_code))]
    fn lookup_team(&self, team: i64, user: i64) -> BoxFuture<'_, Result<Option<Membership<R>>, Error>>;
    fn lookup_team_public(&self, team: Uuid<Team>, user: i64) -> BoxFuture<'_, Result<Option<Membership<R>>, Error>>;
    #[cfg_attr(not(feature = "session"), allow(dead_code))]
    fn fallback_team(&self, user: i64) -> BoxFuture<'_, Result<Option<Membership<R>>, Error>>;
    #[cfg_attr(not(feature = "session"), allow(dead_code))]
    fn saved_team(&self, user: i64) -> BoxFuture<'_, Result<Option<i64>, Error>>;
    #[cfg_attr(not(feature = "session"), allow(dead_code))]
    fn save_team(&self, user: i64, team: Option<i64>) -> BoxFuture<'_, Result<(), Error>>;
}

pub(crate) trait OrgOnly<O>: Send + Sync {
    fn user<'a>(&'a self, parts: &'a mut Parts) -> BoxFuture<'a, Result<i64, Response<Body>>>;
    fn lookup_org_public(&self, org: Uuid<Org>, user: i64) -> BoxFuture<'_, Result<Option<OrgMembership<O>>, Error>>;
}

impl<R: Role, O: OrgRole> TeamOnly<R> for Gate<R, O> {
    fn user<'a>(&'a self, parts: &'a mut Parts) -> BoxFuture<'a, Result<i64, Response<Body>>> {
        self.identify.user_id(parts)
    }
    fn lookup_team(&self, team: i64, user: i64) -> BoxFuture<'_, Result<Option<Membership<R>>, Error>> {
        self.lookup.team(team, user)
    }
    fn lookup_team_public(&self, team: Uuid<Team>, user: i64) -> BoxFuture<'_, Result<Option<Membership<R>>, Error>> {
        self.lookup.team_public(team, user)
    }
    fn fallback_team(&self, user: i64) -> BoxFuture<'_, Result<Option<Membership<R>>, Error>> {
        self.lookup.fallback_team(user)
    }
    fn saved_team(&self, user: i64) -> BoxFuture<'_, Result<Option<i64>, Error>> {
        self.lookup.saved_team(user)
    }
    fn save_team(&self, user: i64, team: Option<i64>) -> BoxFuture<'_, Result<(), Error>> {
        self.lookup.save_team(user, team)
    }
}

impl<R: Role, O: OrgRole> OrgOnly<O> for Gate<R, O> {
    fn user<'a>(&'a self, parts: &'a mut Parts) -> BoxFuture<'a, Result<i64, Response<Body>>> {
        self.identify.user_id(parts)
    }
    fn lookup_org_public(&self, org: Uuid<Org>, user: i64) -> BoxFuture<'_, Result<Option<OrgMembership<O>>, Error>> {
        self.lookup.org_public(org, user)
    }
}

fn not_found() -> Response<Body> {
    StatusCode::NOT_FOUND.into_response()
}

fn store_failed(e: Error) -> Response<Body> {
    tracing::error!("teams: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

fn no_layer() -> Response<Body> {
    tracing::error!("no teams context; add `teams::layer` (after `auth::layer`) to the router");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

#[allow(clippy::result_large_err, reason = "the error is the response itself")]
pub(crate) fn team_gate<R: 'static>(parts: &Parts) -> Result<TeamGate<R>, Response<Body>> {
    parts.extensions.get::<TeamGate<R>>().cloned().ok_or_else(no_layer)
}

/// Give the inputs the store and the current user. Goes after `auth::layer`:
/// `.layer(teams::layer(teams.clone(), teams::auth_user(|u: &User| u.id)))`.
pub fn layer<S: Store>(teams: Teams<S>, identify: impl Identify) -> TeamsLayer<S::Role, S::OrgRole> {
    TeamsLayer(Gate { lookup: Arc::new(teams), identify: Arc::new(identify) })
}

pub struct TeamsLayer<R, O>(Gate<R, O>);

impl<R, O> Clone for TeamsLayer<R, O> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<Inner, R, O> Layer<Inner> for TeamsLayer<R, O> {
    type Service = TeamsService<Inner, R, O>;
    fn layer(&self, inner: Inner) -> Self::Service {
        TeamsService { inner, gate: self.0.clone() }
    }
}

pub struct TeamsService<Inner, R, O> {
    inner: Inner,
    gate: Gate<R, O>,
}

impl<Inner: Clone, R, O> Clone for TeamsService<Inner, R, O> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), gate: self.gate.clone() }
    }
}

impl<Inner, R, O, B> Service<Request<B>> for TeamsService<Inner, R, O>
where
    Inner: Service<Request<B>>,
    R: Role,
    O: OrgRole,
{
    type Response = Inner::Response;
    type Error = Inner::Error;
    type Future = Inner::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        let gate = Arc::new(self.gate.clone());
        req.extensions_mut().insert(TeamGate::<R>(gate.clone()));
        req.extensions_mut().insert(OrgGate::<O>(gate));
        self.inner.call(req)
    }
}

#[derive(Deserialize)]
struct TeamParam {
    team: Uuid<Team>,
}

#[derive(Deserialize)]
struct OrgParam {
    org: Uuid<Org>,
}

/// The current user's access to the `{team}` in the path, a
/// [`Uuid<Team>`](Uuid). `404` if the id does not parse (an internal
/// id), the team does not exist or the user cannot open it, so its
/// existence does not leak.
#[derive(Clone, Debug)]
pub struct Membership<R> {
    pub team: Team,
    pub role: R,
    /// `true` when access comes from an organization role that manages
    /// the team's organization; `role` is then `Role::OWNER`.
    pub via_org: bool,
    pub user_id: i64,
}

impl<R: Role> Membership<R> {
    /// Whether the role has permission `P`.
    pub fn can<P: Permission>(&self) -> bool
    where
        R: Grants<P>,
    {
        self.role.grants()
    }
}

impl<S: Send + Sync, R: Role> FromRequestParts<S> for Membership<R> {
    async fn from_request_parts(parts: &mut Parts, state: &Arc<S>) -> Result<Self, Response<Body>> {
        let gate = team_gate::<R>(parts)?;
        let user = gate.0.user(parts).await?;
        let Path(TeamParam { team }) = Path::<TeamParam>::from_request_parts(parts, state).await?;
        gate.0.lookup_team_public(team, user).await.map_err(store_failed)?.ok_or_else(not_found)
    }
}

impl<S: Send + Sync, R: Role> FromRequest<S> for Membership<R> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}

/// A [`Membership`] whose role has permission `P`; `403` otherwise.
pub struct Can<P, R> {
    pub membership: Membership<R>,
    permission: PhantomData<fn() -> P>,
}

impl<P, R> Deref for Can<P, R> {
    type Target = Membership<R>;
    fn deref(&self) -> &Membership<R> {
        &self.membership
    }
}

impl<S: Send + Sync, P: Permission, R: Grants<P>> FromRequestParts<S> for Can<P, R> {
    async fn from_request_parts(parts: &mut Parts, state: &Arc<S>) -> Result<Self, Response<Body>> {
        let membership = Membership::<R>::from_request_parts(parts, state).await?;
        if !membership.role.grants() {
            return Err(StatusCode::FORBIDDEN.into_response());
        }
        Ok(Can { membership, permission: PhantomData })
    }
}

impl<S: Send + Sync, P: Permission, R: Grants<P>> FromRequest<S> for Can<P, R> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}

/// The current user's role in the `{org}` in the path, a
/// [`Uuid<Org>`](Uuid). `404` if the id does not parse, the
/// organization does not exist or the user is not a member.
#[derive(Clone, Debug)]
pub struct OrgMembership<O> {
    pub org: Org,
    pub role: O,
    pub user_id: i64,
}

impl<O: OrgRole> OrgMembership<O> {
    pub fn can<P: Permission>(&self) -> bool
    where
        O: Grants<P>,
    {
        self.role.grants()
    }
}

impl<S: Send + Sync, O: OrgRole> FromRequestParts<S> for OrgMembership<O> {
    async fn from_request_parts(parts: &mut Parts, state: &Arc<S>) -> Result<Self, Response<Body>> {
        let gate = parts.extensions.get::<OrgGate<O>>().cloned().ok_or_else(no_layer)?;
        let user = gate.0.user(parts).await?;
        let Path(OrgParam { org }) = Path::<OrgParam>::from_request_parts(parts, state).await?;
        gate.0.lookup_org_public(org, user).await.map_err(store_failed)?.ok_or_else(not_found)
    }
}

impl<S: Send + Sync, O: OrgRole> FromRequest<S> for OrgMembership<O> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}

/// An [`OrgMembership`] whose role has permission `P`; `403` otherwise.
pub struct OrgCan<P, O> {
    pub membership: OrgMembership<O>,
    permission: PhantomData<fn() -> P>,
}

impl<P, O> Deref for OrgCan<P, O> {
    type Target = OrgMembership<O>;
    fn deref(&self) -> &OrgMembership<O> {
        &self.membership
    }
}

impl<S: Send + Sync, P: Permission, O: OrgRole + Grants<P>> FromRequestParts<S> for OrgCan<P, O> {
    async fn from_request_parts(parts: &mut Parts, state: &Arc<S>) -> Result<Self, Response<Body>> {
        let membership = OrgMembership::<O>::from_request_parts(parts, state).await?;
        if !membership.role.grants() {
            return Err(StatusCode::FORBIDDEN.into_response());
        }
        Ok(OrgCan { membership, permission: PhantomData })
    }
}

impl<S: Send + Sync, P: Permission, O: OrgRole + Grants<P>> FromRequest<S> for OrgCan<P, O> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}
