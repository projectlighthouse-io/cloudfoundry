//! Know who the current user is, and guard routes.
//!
//! The app implements [`Provider`] (how to load a user by id) and adds
//! [`layer`] after the session layer. Then:
//!
//! - [`Authed<U>`]: handler input that exists only for a logged-in user.
//! - [`Auth<U>`]: optional user, plus `login` and `logout`.
//! - [`require`] / [`guest`]: layers for routes whose handlers take neither.
//! - [`Remember`] + `.remember(codec)`: "remember me" cookies.
//! - [`Tokens`] + `.tokens()`: `Authorization: Bearer` for APIs.
//!
//! Only the user id is stored in the session. The user is loaded at most once
//! per request, and only if something asks.

use std::{
    future::Future,
    ops::Deref,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use securecookie::Codec;

use route::{
    Body, FromRequest, FromRequestParts, IntoResponse, Redirect, UrlFor,
    http::{Extensions, HeaderMap, HeaderValue, Request, Response, StatusCode, header, request::Parts},
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

/// Providers that keep a remember token per user, for "remember me".
pub trait Remember: Provider {
    /// The token stored for `user`, if any.
    fn remember_token(&self, user: &Self::User) -> Option<String>;
    /// Store a new token for the user with `id`, or clear it with `None`.
    fn set_remember_token(&self, id: Self::Id, token: Option<String>) -> impl Future<Output = Result<(), Error>> + Send;
}

/// Providers that can find a user by API token, for `Authorization: Bearer`.
pub trait Tokens: Provider {
    fn by_token(&self, token: &str) -> impl Future<Output = Result<Option<Self::User>, Error>> + Send;
}

trait ErasedRemember<U>: Send + Sync {
    fn token_of(&self, user: &U) -> Option<String>;
    fn set(&self, id: Value, token: Option<String>) -> BoxFuture<'_, Result<(), Error>>;
}

impl<P: Remember> ErasedRemember<P::User> for P {
    fn token_of(&self, user: &P::User) -> Option<String> {
        self.remember_token(user)
    }

    fn set(&self, id: Value, token: Option<String>) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move { self.set_remember_token(serde_json::from_value(id)?, token).await })
    }
}

trait ErasedTokens<U>: Send + Sync {
    fn by_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<Option<U>, Error>>;
}

impl<P: Tokens> ErasedTokens<P::User> for P {
    fn by_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<Option<P::User>, Error>> {
        Box::pin(Tokens::by_token(self, token))
    }
}

/// Browsers cap cookie lifetimes at 400 days.
const REMEMBER_FOR: Duration = Duration::from_secs(400 * 86400);

struct RememberCfg<U> {
    store: Arc<dyn ErasedRemember<U>>,
    codec: Arc<Codec>,
    cookie: String,
}

/// Cookie work the layer does after the handler.
#[derive(Default)]
enum Pending {
    #[default]
    None,
    Remember { id: Value, token: String },
    Forget { id: Option<Value> },
}

/// Per-request auth context, in request extensions.
struct AuthCx<U> {
    provider: Arc<dyn Erased<U>>,
    remember: Option<Arc<RememberCfg<U>>>,
    tokens: Option<Arc<dyn ErasedTokens<U>>>,
    cfg: Arc<Config>,
    user: Arc<OnceCell<Option<U>>>,
    bearer: Option<Arc<str>>,
    recaller: Option<Arc<str>>,
    pending: Arc<Mutex<Pending>>,
}

impl<U> Clone for AuthCx<U> {
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            remember: self.remember.clone(),
            tokens: self.tokens.clone(),
            cfg: self.cfg.clone(),
            user: self.user.clone(),
            bearer: self.bearer.clone(),
            recaller: self.recaller.clone(),
            pending: self.pending.clone(),
        }
    }
}

struct Config {
    login_route: &'static str,
    secure: bool,
}

impl<U: Clone + Send + Sync + 'static> AuthCx<U> {
    /// Bearer token first (APIs), then the session, then the remember cookie.
    async fn user(&self, session: Option<&Session>) -> Result<Option<U>, Error> {
        self.user
            .get_or_try_init(|| async {
                if let (Some(tokens), Some(bearer)) = (&self.tokens, &self.bearer) {
                    return tokens.by_token(bearer).await;
                }
                let Some(session) = session else { return Ok(None) };
                match session.get::<Value>(SESSION_KEY)? {
                    Some(id) => self.provider.find(id).await,
                    None => self.recall(session).await,
                }
            })
            .await
            .cloned()
    }

    /// Log in from a valid remember cookie whose token still matches the stored one.
    async fn recall(&self, session: &Session) -> Result<Option<U>, Error> {
        let (Some(r), Some(cookie)) = (&self.remember, &self.recaller) else { return Ok(None) };
        let mut found = None;
        if let Ok((id, token)) = r.codec.verify::<(Value, String)>(&r.cookie, cookie, REMEMBER_FOR) {
            if let Some(user) = self.provider.find(id.clone()).await? {
                if r.store.token_of(&user).is_some_and(|t| ct_eq(&t, &token)) {
                    found = Some((id, user));
                }
            }
        }
        match found {
            Some((id, user)) => {
                session.regenerate();
                session.insert(SESSION_KEY, &id)?;
                Ok(Some(user))
            }
            None => {
                // Stale or forged: drop the cookie.
                *self.pending.lock().unwrap() = Pending::Forget { id: None };
                Ok(None)
            }
        }
    }

    async fn finish<B>(&self, res: &mut Response<B>) {
        let Some(r) = &self.remember else { return };
        let pending = std::mem::take(&mut *self.pending.lock().unwrap());
        match pending {
            Pending::None => {}
            Pending::Remember { id, token } => {
                if let Err(e) = r.store.set(id.clone(), Some(token.clone())).await {
                    return tracing::error!("storing the remember token failed: {e}");
                }
                match r.codec.sign(&r.cookie, &(id, token)) {
                    Ok(v) => set_cookie(res, &r.cookie, &v, REMEMBER_FOR.as_secs(), self.cfg.secure),
                    Err(e) => tracing::error!("signing the remember cookie failed: {e}"),
                }
            }
            Pending::Forget { id } => {
                if let Some(id) = id {
                    if let Err(e) = r.store.set(id, None).await {
                        tracing::error!("clearing the remember token failed: {e}");
                    }
                }
                set_cookie(res, &r.cookie, "", 0, self.cfg.secure);
            }
        }
    }
}

fn set_cookie<B>(res: &mut Response<B>, name: &str, value: &str, max_age: u64, secure: bool) {
    let secure = if secure { "; Secure" } else { "" };
    let c = format!("{name}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}");
    if let Ok(v) = HeaderValue::try_from(c) {
        res.headers_mut().append(header::SET_COOKIE, v);
    }
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ").map(str::trim)
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random_token() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("OS random source");
    URL_SAFE_NO_PAD.encode(b)
}

fn context<U: Send + Sync + 'static>(ext: &Extensions) -> Result<(AuthCx<U>, Option<Session>), Response<Body>> {
    match ext.get::<AuthCx<U>>() {
        Some(cx) => Ok((cx.clone(), ext.get::<Session>().cloned())),
        None => {
            tracing::error!("no auth context; add `auth::layer` (after `session::layer`) to the router");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

async fn current<U: Clone + Send + Sync + 'static>(ext: &Extensions) -> Result<(Option<U>, AuthCx<U>, Option<Session>), Response<Body>> {
    let (cx, session) = context::<U>(ext)?;
    match cx.user(session.as_ref()).await {
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

/// Make `Auth<U>`, `Authed<U>`, `require` and `guest` work. Add after the
/// session layer.
pub fn layer<P: Provider>(provider: P) -> AuthLayer<P> {
    AuthLayer { provider: Arc::new(provider), remember: None, tokens: None, cfg: Arc::new(Config { login_route: "login", secure: true }) }
}

pub struct AuthLayer<P: Provider> {
    provider: Arc<P>,
    remember: Option<Arc<RememberCfg<P::User>>>,
    tokens: Option<Arc<dyn ErasedTokens<P::User>>>,
    cfg: Arc<Config>,
}

impl<P: Provider> AuthLayer<P> {
    /// Route name anonymous page visits are sent to. Default `login`.
    pub fn login_route(self, name: &'static str) -> Self {
        Self { cfg: Arc::new(Config { login_route: name, secure: self.cfg.secure }), ..self }
    }

    /// `Secure` on the remember cookie. Default `true` (browsers accept it on localhost).
    pub fn secure(self, secure: bool) -> Self {
        Self { cfg: Arc::new(Config { login_route: self.cfg.login_route, secure }), ..self }
    }
}

impl<P: Remember> AuthLayer<P> {
    /// Enable "remember me": `Auth::login_remember` sets a signed, 400-day
    /// cookie that logs the user back in after the session ends. Logout
    /// clears the stored token, so old cookies stop working.
    pub fn remember(mut self, codec: Codec) -> Self {
        let store: Arc<dyn ErasedRemember<P::User>> = self.provider.clone();
        self.remember = Some(Arc::new(RememberCfg { store, codec: Arc::new(codec), cookie: "remember".into() }));
        self
    }
}

impl<P: Tokens> AuthLayer<P> {
    /// Accept `Authorization: Bearer <token>`. A bearer request is
    /// authenticated by the token alone; the session is not consulted. Exempt
    /// such API routes from CSRF, which guards cookie-based requests.
    pub fn tokens(mut self) -> Self {
        let tokens: Arc<dyn ErasedTokens<P::User>> = self.provider.clone();
        self.tokens = Some(tokens);
        self
    }
}

impl<P: Provider> Clone for AuthLayer<P> {
    fn clone(&self) -> Self {
        Self { provider: self.provider.clone(), remember: self.remember.clone(), tokens: self.tokens.clone(), cfg: self.cfg.clone() }
    }
}

impl<S, P: Provider> Layer<S> for AuthLayer<P> {
    type Service = AuthService<S, P>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService { inner, layer: self.clone() }
    }
}

pub struct AuthService<S, P: Provider> {
    inner: S,
    layer: AuthLayer<P>,
}

impl<S: Clone, P: Provider> Clone for AuthService<S, P> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), layer: self.layer.clone() }
    }
}

impl<S, P, ReqB, ResB> Service<Request<ReqB>> for AuthService<S, P>
where
    S: Service<Request<ReqB>, Response = Response<ResB>> + Clone + Send + 'static,
    S::Future: Send,
    P: Provider,
    ReqB: Send + 'static,
    ResB: Send + 'static,
{
    type Response = Response<ResB>;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Response<ResB>, S::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqB>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let l = &self.layer;
        let provider: Arc<dyn Erased<P::User>> = l.provider.clone();
        let cx = AuthCx::<P::User> {
            provider,
            remember: l.remember.clone(),
            tokens: l.tokens.clone(),
            cfg: l.cfg.clone(),
            user: Arc::new(OnceCell::new()),
            bearer: l.tokens.as_ref().and_then(|_| bearer(req.headers())).map(Into::into),
            recaller: l.remember.as_ref().and_then(|r| cookie(req.headers(), &r.cookie)).map(Into::into),
            pending: Arc::default(),
        };
        req.extensions_mut().insert(cx.clone());
        Box::pin(async move {
            let mut res = inner.call(req).await?;
            cx.finish(&mut res).await;
            Ok(res)
        })
    }
}

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

/// The user if there is one, plus login and logout. Needs the session layer.
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

    /// `login`, plus a remember cookie (needs `.remember(codec)` on the layer).
    pub fn login_remember(&self, user: &U) {
        self.login(user);
        if self.cx.remember.is_none() {
            return tracing::warn!("login_remember without `.remember(codec)` on the auth layer; no cookie set");
        }
        *self.cx.pending.lock().unwrap() = Pending::Remember { id: self.cx.provider.id(user), token: random_token() };
    }

    /// Log out. Invalidates the whole session and, with remember enabled,
    /// clears the stored token and the cookie.
    pub fn logout(&self) {
        self.session.invalidate();
        if self.cx.remember.is_some() {
            let id = self.user.as_ref().map(|u| self.cx.provider.id(u));
            *self.cx.pending.lock().unwrap() = Pending::Forget { id };
        }
    }
}

impl<S: Send + Sync, U: Clone + Send + Sync + 'static> FromRequestParts<S> for Auth<U> {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        let (user, cx, session) = current::<U>(&parts.extensions).await?;
        let Some(session) = session else {
            tracing::error!("`Auth` needs a session; add `session::layer` before `auth::layer`");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        };
        Ok(Auth { user, cx, session })
    }
}

impl<S: Send + Sync, U: Clone + Send + Sync + 'static> FromRequest<S> for Auth<U> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}

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
