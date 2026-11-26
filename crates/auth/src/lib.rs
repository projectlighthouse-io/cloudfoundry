//! Know who the current user is, and guard routes.
//!
//! The app implements [`Provider`] (how to load a user by id) and adds
//! [`layer`] after the session layer. Then:
//!
//! - [`Authed<U>`]: handler input that exists only for a logged-in user.
//! - [`Auth<U>`]: optional user, plus `login` and `logout`.
//! - [`require`] / [`guest`]: layers for routes whose handlers take neither.
//!
//! Only the user id is stored in the session. The user is loaded at most once
//! per request, and only if something asks.

use std::{
    future::Future,
    ops::Deref,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use route::{
    Body, FromRequest, FromRequestParts, IntoResponse, Redirect, UrlFor,
    http::{Extensions, HeaderMap, Request, Response, StatusCode, header, request::Parts},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use session::Session;
use tokio::sync::OnceCell;
use tower::{Layer, Service};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

const SESSION_KEY: &str = "_auth_id";

/// How the app loads its users. The only integration point.
pub trait Provider: Send + Sync + 'static {
    type User: Clone + Send + Sync + 'static;
    type Id: Serialize + DeserializeOwned + Send;

    fn find(&self, id: Self::Id) -> impl Future<Output = Result<Option<Self::User>, Error>> + Send;
    fn id(&self, user: &Self::User) -> Self::Id;
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// `Provider` with the id type erased to JSON, so lookups go by user type alone.
trait Erased<U>: Send + Sync {
    fn find(&self, id: Value) -> BoxFuture<'_, Result<Option<U>, Error>>;
    fn id(&self, user: &U) -> Value;
}

impl<P: Provider> Erased<P::User> for P {
    fn find(&self, id: Value) -> BoxFuture<'_, Result<Option<P::User>, Error>> {
        Box::pin(async move {
            // An id that no longer parses (the id type changed) is a logged-out user.
            match serde_json::from_value(id) {
                Ok(id) => Provider::find(self, id).await,
                Err(_) => Ok(None),
            }
        })
    }

    fn id(&self, user: &P::User) -> Value {
        serde_json::to_value(Provider::id(self, user)).expect("user id serializes to JSON")
    }
}

/// Per-request auth context, in request extensions.
struct AuthCx<U> {
    provider: Arc<dyn Erased<U>>,
    user: Arc<OnceCell<Option<U>>>,
    cfg: Arc<Config>,
}

impl<U> Clone for AuthCx<U> {
    fn clone(&self) -> Self {
        Self { provider: self.provider.clone(), user: self.user.clone(), cfg: self.cfg.clone() }
    }
}

struct Config {
    login_route: &'static str,
}

impl<U: Clone + Send + Sync + 'static> AuthCx<U> {
    async fn user(&self, session: &Session) -> Result<Option<U>, Error> {
        self.user
            .get_or_try_init(|| async {
                match session.get::<Value>(SESSION_KEY)? {
                    Some(id) => self.provider.find(id).await,
                    None => Ok(None),
                }
            })
            .await
            .cloned()
    }
}

fn context<U: Send + Sync + 'static>(ext: &Extensions) -> Result<(AuthCx<U>, Session), Response<Body>> {
    match (ext.get::<AuthCx<U>>(), ext.get::<Session>()) {
        (Some(cx), Some(s)) => Ok((cx.clone(), s.clone())),
        _ => {
            tracing::error!("no auth context; add `session::layer` and then `auth::layer` to the router");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

async fn current<U: Clone + Send + Sync + 'static>(ext: &Extensions) -> Result<(Option<U>, AuthCx<U>, Session), Response<Body>> {
    let (cx, session) = context::<U>(ext)?;
    match cx.user(&session).await {
        Ok(user) => Ok((user, cx, session)),
        Err(e) => {
            tracing::error!("loading the current user failed: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

/// Redirect page visits to the login route; `401` for JSON clients.
fn unauthenticated(ext: &Extensions, headers: &HeaderMap, login: &'static str) -> Response<Body> {
    let inertia = headers.contains_key("x-inertia");
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"));
    let url = ext.get::<UrlFor>().and_then(|u| u.url(login, &[]).ok());
    match url {
        Some(url) if inertia || !wants_json => Redirect::to(url).into_response(),
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

// ---- layer ----

/// Make `Auth<U>`, `Authed<U>`, `require` and `guest` work. Add after the
/// session layer.
pub fn layer<P: Provider>(provider: P) -> AuthLayer<P::User> {
    AuthLayer { provider: Arc::new(provider), cfg: Arc::new(Config { login_route: "login" }) }
}

pub struct AuthLayer<U> {
    provider: Arc<dyn Erased<U>>,
    cfg: Arc<Config>,
}

impl<U> AuthLayer<U> {
    /// Route name anonymous page visits are sent to. Default `login`.
    pub fn login_route(self, name: &'static str) -> Self {
        Self { cfg: Arc::new(Config { login_route: name }), ..self }
    }
}

impl<U> Clone for AuthLayer<U> {
    fn clone(&self) -> Self {
        Self { provider: self.provider.clone(), cfg: self.cfg.clone() }
    }
}

impl<S, U> Layer<S> for AuthLayer<U> {
    type Service = AuthService<S, U>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService { inner, layer: self.clone() }
    }
}

pub struct AuthService<S, U> {
    inner: S,
    layer: AuthLayer<U>,
}

impl<S: Clone, U> Clone for AuthService<S, U> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), layer: self.layer.clone() }
    }
}

impl<S, U, B> Service<Request<B>> for AuthService<S, U>
where
    S: Service<Request<B>>,
    U: Send + Sync + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> S::Future {
        req.extensions_mut().insert(AuthCx::<U> {
            provider: self.layer.provider.clone(),
            user: Arc::new(OnceCell::new()),
            cfg: self.layer.cfg.clone(),
        });
        self.inner.call(req)
    }
}

// ---- inputs ----

/// The logged-in user. A handler taking one never runs for an anonymous
/// request, even on a route without `require`.
pub struct Authed<U>(pub U);

impl<U> Deref for Authed<U> {
    type Target = U;
    fn deref(&self) -> &U {
        &self.0
    }
}

impl<S: Send + Sync, U: Clone + Send + Sync + 'static> FromRequestParts<S> for Authed<U> {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        match current::<U>(&parts.extensions).await? {
            (Some(user), ..) => Ok(Authed(user)),
            (None, cx, _) => Err(unauthenticated(&parts.extensions, &parts.headers, cx.cfg.login_route)),
        }
    }
}

impl<S: Send + Sync, U: Clone + Send + Sync + 'static> FromRequest<S> for Authed<U> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}

/// The user if there is one, plus login and logout.
pub struct Auth<U> {
    user: Option<U>,
    cx: AuthCx<U>,
    session: Session,
}

impl<U> Auth<U> {
    pub fn user(&self) -> Option<&U> {
        self.user.as_ref()
    }

    /// Log `user` in. Regenerates the session id.
    pub fn login(&self, user: &U) {
        self.session.regenerate();
        self.session.insert(SESSION_KEY, &self.cx.provider.id(user)).expect("JSON value serializes");
    }

    /// Log out. Invalidates the whole session.
    pub fn logout(&self) {
        self.session.invalidate();
    }
}

impl<S: Send + Sync, U: Clone + Send + Sync + 'static> FromRequestParts<S> for Auth<U> {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        let (user, cx, session) = current::<U>(&parts.extensions).await?;
        Ok(Auth { user, cx, session })
    }
}

impl<S: Send + Sync, U: Clone + Send + Sync + 'static> FromRequest<S> for Auth<U> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}

// ---- guards ----

/// Only logged-in users pass. Others get the login redirect or a `401`.
pub fn require<U>() -> Guard<U> {
    Guard { kind: Kind::Require, _user: std::marker::PhantomData }
}

/// Only anonymous users pass. Logged-in users are redirected to `home`.
pub fn guest<U>() -> Guard<U> {
    Guard { kind: Kind::Guest("home"), _user: std::marker::PhantomData }
}

#[derive(Clone, Copy)]
enum Kind {
    Require,
    Guest(&'static str),
}

pub struct Guard<U> {
    kind: Kind,
    _user: std::marker::PhantomData<fn() -> U>,
}

impl<U> Guard<U> {
    /// For `guest`: where logged-in users go. Default `home`.
    pub fn redirect_to(self, route: &'static str) -> Self {
        Guard { kind: Kind::Guest(route), _user: std::marker::PhantomData }
    }
}

impl<U> Clone for Guard<U> {
    fn clone(&self) -> Self {
        Guard { kind: self.kind, _user: std::marker::PhantomData }
    }
}

impl<S, U> Layer<S> for Guard<U> {
    type Service = GuardService<S, U>;

    fn layer(&self, inner: S) -> Self::Service {
        GuardService { inner, guard: self.clone() }
    }
}

pub struct GuardService<S, U> {
    inner: S,
    guard: Guard<U>,
}

impl<S: Clone, U> Clone for GuardService<S, U> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), guard: self.guard.clone() }
    }
}

impl<S, U, ReqB> Service<Request<ReqB>> for GuardService<S, U>
where
    S: Service<Request<ReqB>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send,
    U: Clone + Send + Sync + 'static,
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
        let kind = self.guard.kind;
        Box::pin(async move {
            let (user, cx, _) = match current::<U>(req.extensions()).await {
                Ok(c) => c,
                Err(res) => return Ok(res),
            };
            match (kind, user.is_some()) {
                (Kind::Require, false) => Ok(unauthenticated(req.extensions(), req.headers(), cx.cfg.login_route)),
                (Kind::Guest(home), true) => Ok(match req.extensions().get::<UrlFor>().and_then(|u| u.url(home, &[]).ok()) {
                    Some(url) => Redirect::to(url).into_response(),
                    None => StatusCode::FORBIDDEN.into_response(),
                }),
                _ => inner.call(req).await,
            }
        })
    }
}
