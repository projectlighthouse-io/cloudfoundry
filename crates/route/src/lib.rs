//! Match a request to a handler.
//!
//! ```ignore
//! let mut r = Router::new();
//! r.get("/", home).name("home");
//! r.group("/runs", |r| {
//!     r.name_prefix("runs.");
//!     r.get("/{id}", show).name("show");
//! });
//! let routes = r.build(state)?;
//! ```

mod extract;
mod response;
mod url;

#[cfg(feature = "action")]
mod action_handler;
#[cfg(feature = "command")]
pub mod command;
#[cfg(feature = "multipart")]
mod multipart;
#[cfg(feature = "server")]
mod server;

#[cfg(feature = "action")]
pub use action_handler::{ActionHandler, action};
pub use extract::{BodyLimit, Cx, DEFAULT_BODY_LIMIT, Form, FromRequest, FromRequestParts, Json, Path, Query, from_parts};
pub use http;
pub use response::{Html, IntoResponse, Redirect};
#[cfg(feature = "multipart")]
pub use multipart::{Field, Multipart, MultipartError};
#[cfg(feature = "server")]
pub use server::serve;
pub use url::UrlError;

use std::{
    any::type_name,
    collections::HashMap,
    convert::Infallible,
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use bytes::Bytes;
use http::{HeaderValue, Method, Request, Response, StatusCode, header};
use http_body_util::{BodyExt, Empty, Full};
use tower::{
    Layer, Service, ServiceExt,
    util::{BoxCloneSyncService, Oneshot},
};

use crate::{extract::PathParams, response::PendingRoute, url::Urls};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
/// Request and response body.
pub type Body = http_body_util::combinators::UnsyncBoxBody<Bytes, BoxError>;
/// A route's composed service: handler plus its layers.
pub type BoxService = BoxCloneSyncService<Request<Body>, Response<Body>, Infallible>;

pub fn body(b: impl Into<Bytes>) -> Body {
    Full::new(b.into()).map_err(|e| match e {}).boxed_unsync()
}

pub fn empty() -> Body {
    Empty::new().map_err(|e| match e {}).boxed_unsync()
}

/// A route name. `&'static str` is one; an app enum can be one too, so a
/// mistyped name fails to compile.
pub trait Name: Copy + 'static {
    /// Every name. `Router::build_named` fails if one was never registered.
    const ALL: &'static [Self];
    fn as_str(&self) -> &'static str;
}

impl Name for &'static str {
    const ALL: &'static [Self] = &[];
    fn as_str(&self) -> &'static str {
        self
    }
}

/// A free async function: `fn(I) -> O` or `fn(Cx<S>, I) -> O`, where `I`
/// implements [`FromRequest`] and `O` implements [`IntoResponse`].
pub trait Handler<S, M>: Clone + Send + Sync + 'static {
    fn call(&self, req: Request<Body>, state: Arc<S>) -> impl Future<Output = Response<Body>> + Send;
}

impl<S, F, Fut, I, O> Handler<S, (I,)> for F
where
    S: Send + Sync + 'static,
    F: Fn(I) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = O> + Send,
    I: FromRequest<S>,
    O: IntoResponse,
{
    fn call(&self, req: Request<Body>, state: Arc<S>) -> impl Future<Output = Response<Body>> + Send {
        async move {
            match I::from_request(req, &state).await {
                Ok(input) => self(input).await.into_response(),
                Err(rejection) => rejection,
            }
        }
    }
}

impl<S, F, Fut, I, O> Handler<S, (Cx<S>, I)> for F
where
    S: Send + Sync + 'static,
    F: Fn(Cx<S>, I) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = O> + Send,
    I: FromRequest<S>,
    O: IntoResponse,
{
    fn call(&self, req: Request<Body>, state: Arc<S>) -> impl Future<Output = Response<Body>> + Send {
        async move {
            match I::from_request(req, &state).await {
                Ok(input) => self(Cx(state), input).await.into_response(),
                Err(rejection) => rejection,
            }
        }
    }
}

type MakeService<S> = Arc<dyn Fn(Arc<S>, Arc<Urls>) -> BoxService + Send + Sync>;

#[derive(Clone)]
struct LayerFn {
    name: &'static str,
    apply: Arc<dyn Fn(BoxService) -> BoxService + Send + Sync>,
}

/// A registered route. Returned by `get`, `post`, ... so it can be named.
pub struct Route<S> {
    method: Method,
    path: String,
    name: Option<&'static str>,
    body_limit: Option<usize>,
    make: MakeService<S>,
}

impl<S> Route<S> {
    pub fn name(&mut self, name: impl Name) -> &mut Self {
        self.name = Some(name.as_str());
        self
    }

    /// Largest request body this route's inputs read, in bytes. Overrides
    /// the router's and group's limit.
    pub fn body_limit(&mut self, bytes: usize) -> &mut Self {
        self.body_limit = Some(bytes);
        self
    }
}

/// Route table under construction. Immutable once built into [`Routes`].
pub struct Router<S> {
    prefix: String,
    name_prefix: String,
    layers: Vec<LayerFn>,
    routes: Vec<Route<S>>,
    groups: Vec<Router<S>>,
    fallback: Option<BoxService>,
    body_limit: Option<usize>,
}

impl<S> Default for Router<S> {
    fn default() -> Self {
        Self { prefix: String::new(), name_prefix: String::new(), layers: vec![], routes: vec![], groups: vec![], fallback: None, body_limit: None }
    }
}

impl<S: Send + Sync + 'static> Router<S> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn route<H, M>(&mut self, method: Method, path: &str, handler: H) -> &mut Route<S>
    where
        H: Handler<S, M>,
        M: 'static,
    {
        let make: MakeService<S> = Arc::new(move |state, urls| handler_service(handler.clone(), state, urls));
        self.routes.push(Route { method, path: path.to_owned(), name: None, body_limit: None, make });
        self.routes.last_mut().unwrap()
    }

    pub fn get<H: Handler<S, M>, M: 'static>(&mut self, path: &str, h: H) -> &mut Route<S> {
        self.route(Method::GET, path, h)
    }
    pub fn post<H: Handler<S, M>, M: 'static>(&mut self, path: &str, h: H) -> &mut Route<S> {
        self.route(Method::POST, path, h)
    }
    pub fn put<H: Handler<S, M>, M: 'static>(&mut self, path: &str, h: H) -> &mut Route<S> {
        self.route(Method::PUT, path, h)
    }
    pub fn patch<H: Handler<S, M>, M: 'static>(&mut self, path: &str, h: H) -> &mut Route<S> {
        self.route(Method::PATCH, path, h)
    }
    pub fn delete<H: Handler<S, M>, M: 'static>(&mut self, path: &str, h: H) -> &mut Route<S> {
        self.route(Method::DELETE, path, h)
    }

    /// Routes under a shared path prefix. Layers and the name prefix set
    /// inside apply to the group only.
    pub fn group(&mut self, prefix: &str, f: impl FnOnce(&mut Router<S>)) -> &mut Self {
        let mut group = Router { prefix: prefix.to_owned(), ..Router::default() };
        f(&mut group);
        self.groups.push(group);
        self
    }

    /// Largest request body the inputs of this router's (or group's) routes
    /// read, in bytes. Nested groups inherit it. Default `DEFAULT_BODY_LIMIT` (2 MiB).
    pub fn body_limit(&mut self, bytes: usize) -> &mut Self {
        self.body_limit = Some(bytes);
        self
    }

    pub fn name_prefix(&mut self, prefix: &str) -> &mut Self {
        self.name_prefix = prefix.to_owned();
        self
    }

    /// A tower layer for every route in this router and its groups. The first
    /// layer added is the outermost.
    pub fn layer<L>(&mut self, layer: L) -> &mut Self
    where
        L: Layer<BoxService> + Send + Sync + 'static,
        L::Service: Service<Request<Body>, Response = Response<Body>, Error = Infallible> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request<Body>>>::Future: Send + 'static,
    {
        self.layers.push(LayerFn {
            name: short_name(type_name::<L>()),
            apply: Arc::new(move |svc| BoxCloneSyncService::new(layer.layer(svc))),
        });
        self
    }

    /// Serve requests that match no path, e.g. `tower_http::services::ServeDir`
    /// for built assets. Router layers do not wrap it. Without one, a 404.
    pub fn fallback<T, B>(&mut self, svc: T) -> &mut Self
    where
        T: Service<Request<Body>, Response = Response<B>, Error = Infallible> + Clone + Send + Sync + 'static,
        T::Future: Send + 'static,
        B: http_body::Body<Data = Bytes> + Send + 'static,
        B::Error: Into<BoxError>,
    {
        let svc = svc.map_response(|res: Response<B>| res.map(|b| b.map_err(Into::into).boxed_unsync()));
        self.fallback = Some(BoxCloneSyncService::new(svc));
        self
    }

    /// Validate the table and freeze it. Fails on duplicate names, duplicate
    /// method and path pairs, and conflicting paths.
    pub fn build(&self, state: S) -> Result<Routes, BuildError> {
        let mut flat = vec![];
        self.flatten("", "", &[], DEFAULT_BODY_LIMIT, &mut flat);

        let mut names = HashMap::new();
        for f in &flat {
            if !f.path.starts_with('/') {
                return Err(BuildError::InvalidPath(f.path.clone()));
            }
            if let Some(name) = &f.name {
                if names.insert(name.clone(), f.path.clone()).is_some() {
                    return Err(BuildError::DuplicateName(name.clone()));
                }
            }
        }
        let urls = Arc::new(Urls(names));
        let state = Arc::new(state);

        let mut matcher = matchit::Router::new();
        let mut by_path: HashMap<String, usize> = HashMap::new();
        let mut endpoints: Vec<Vec<(Method, BoxService, usize)>> = vec![];
        let mut table = vec![];

        for f in flat {
            // Composed once here, not walked per request.
            // ponytail: one box per layer since layers are stored type-erased;
            // a typed stack would need the layer list in the router's type.
            let mut svc = (f.make)(state.clone(), urls.clone());
            for l in f.layers.iter().rev() {
                svc = (l.apply)(svc);
            }

            let ep = match by_path.get(&f.path) {
                Some(&ep) => ep,
                None => {
                    matcher
                        .insert(f.path.clone(), endpoints.len())
                        .map_err(|e| BuildError::Conflict { path: f.path.clone(), reason: e.to_string() })?;
                    by_path.insert(f.path.clone(), endpoints.len());
                    endpoints.push(vec![]);
                    endpoints.len() - 1
                }
            };
            if endpoints[ep].iter().any(|(m, ..)| *m == f.method) {
                return Err(BuildError::DuplicateRoute(f.method, f.path));
            }
            endpoints[ep].push((f.method.clone(), svc, table.len()));
            table.push(Arc::new(RouteInfo {
                method: f.method,
                path: f.path,
                name: f.name,
                layers: f.layers.iter().map(|l| l.name).collect(),
                body_limit: f.body_limit,
            }));
        }

        Ok(Routes(Arc::new(Inner { matcher, endpoints, table, urls, fallback: self.fallback.clone() })))
    }

    /// `build`, then fail if any name in `N::ALL` has no route.
    pub fn build_named<N: Name>(&self, state: S) -> Result<Routes, BuildError> {
        let routes = self.build(state)?;
        match N::ALL.iter().find(|n| !routes.0.urls.0.contains_key(n.as_str())) {
            Some(n) => Err(BuildError::UnregisteredName(n.as_str())),
            None => Ok(routes),
        }
    }

    fn flatten(&self, prefix: &str, name_prefix: &str, layers: &[LayerFn], body_limit: usize, out: &mut Vec<Flat<S>>) {
        let body_limit = self.body_limit.unwrap_or(body_limit);
        // A trailing slash on a group prefix (`/` or `/api/`) must not double up.
        let prefix = format!("{prefix}{}", self.prefix.trim_end_matches('/'));
        let name_prefix = format!("{name_prefix}{}", self.name_prefix);
        let layers: Vec<LayerFn> = layers.iter().chain(&self.layers).cloned().collect();
        for r in &self.routes {
            out.push(Flat {
                method: r.method.clone(),
                path: join(&prefix, &r.path),
                name: r.name.map(|n| format!("{name_prefix}{n}")),
                layers: layers.clone(),
                body_limit: r.body_limit.unwrap_or(body_limit),
                make: r.make.clone(),
            });
        }
        for g in &self.groups {
            g.flatten(&prefix, &name_prefix, &layers, body_limit, out);
        }
    }
}

struct Flat<S> {
    method: Method,
    path: String,
    name: Option<String>,
    layers: Vec<LayerFn>,
    body_limit: usize,
    make: MakeService<S>,
}

/// `session::layer::SessionLayer<session::store::PostgresStore>` -> `SessionLayer`.
fn short_name(full: &'static str) -> &'static str {
    let base = full.split('<').next().unwrap_or(full);
    base.rsplit("::").next().unwrap_or(base)
}

fn join(prefix: &str, path: &str) -> String {
    let p = format!("{prefix}{path}");
    match p.trim_end_matches('/') {
        "" => "/".to_owned(),
        t => t.to_owned(),
    }
}

fn handler_service<S, H, M>(handler: H, state: Arc<S>, urls: Arc<Urls>) -> BoxService
where
    S: Send + Sync + 'static,
    H: Handler<S, M>,
{
    BoxCloneSyncService::new(tower::service_fn(move |req| {
        let (handler, state, urls) = (handler.clone(), state.clone(), urls.clone());
        async move {
            let mut res = handler.call(req, state).await;
            // Resolved here, inside the layers, so layers see a real Location.
            if let Some(PendingRoute(name, params)) = res.extensions_mut().remove::<PendingRoute>() {
                let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                match urls.url(name, &params).ok().and_then(|u| HeaderValue::try_from(u).ok()) {
                    Some(loc) => {
                        res.headers_mut().insert(header::LOCATION, loc);
                    }
                    None => {
                        tracing::error!("redirect to route `{name}` failed: unknown name or wrong parameters");
                        res = StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                }
            }
            Ok::<_, Infallible>(res)
        }
    }))
}

/// One row of the route table, as `art route:list` shows it.
#[derive(Debug)]
pub struct RouteInfo {
    pub method: Method,
    pub path: String,
    pub name: Option<String>,
    pub layers: Vec<&'static str>,
    /// Largest body this route's inputs read, in bytes.
    pub body_limit: usize,
}

/// The matched route. In request extensions, for layers such as CSRF that
/// exempt routes by name.
#[derive(Clone, Debug)]
pub struct Matched(pub Arc<RouteInfo>);

/// The peer address of the connection. `serve` puts it in request
/// extensions. Behind a proxy it is the proxy's address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientAddr(pub std::net::SocketAddr);

struct Inner {
    matcher: matchit::Router<usize>,
    endpoints: Vec<Vec<(Method, BoxService, usize)>>,
    table: Vec<Arc<RouteInfo>>,
    urls: Arc<Urls>,
    fallback: Option<BoxService>,
}

/// URL generation for layers, which run outside the handler. In request
/// extensions for every matched request.
#[derive(Clone)]
pub struct UrlFor(Arc<Urls>);

impl UrlFor {
    pub fn url(&self, name: impl Name, params: &[(&str, &str)]) -> Result<String, UrlError> {
        self.0.url(name.as_str(), params)
    }
}

/// A built, immutable router. Cheap to clone. Lookup takes no lock.
#[derive(Clone)]
pub struct Routes(Arc<Inner>);

impl Routes {
    pub fn table(&self) -> impl Iterator<Item = &RouteInfo> {
        self.0.table.iter().map(|r| &**r)
    }

    /// URL for a named route. Errors on an unknown name, a missing parameter
    /// or an extra one.
    pub fn url(&self, name: impl Name, params: &[(&str, &str)]) -> Result<String, UrlError> {
        self.0.urls.url(name.as_str(), params)
    }

    pub async fn handle(&self, req: Request<Body>) -> Response<Body> {
        match self.dispatch(req).await {
            Ok(res) => res,
            Err(e) => match e {},
        }
    }

    fn dispatch(&self, mut req: Request<Body>) -> ResponseFuture {
        let inner = &self.0;
        let Ok(m) = inner.matcher.at(req.uri().path()) else {
            return match &inner.fallback {
                Some(svc) => ResponseFuture(Kind::Service(svc.clone().oneshot(req))),
                None => ResponseFuture::ready(StatusCode::NOT_FOUND.into_response()),
            };
        };
        let ep = *m.value;
        let params = PathParams::decode(m.params.iter());

        let methods = &inner.endpoints[ep];
        let hit = methods.iter().find(|(m, ..)| m == req.method()).or_else(|| {
            (req.method() == Method::HEAD).then(|| methods.iter().find(|(m, ..)| m == Method::GET)).flatten()
        });
        let Some((_, svc, t)) = hit else {
            let allow: Vec<&str> = methods.iter().map(|(m, ..)| m.as_str()).collect();
            let mut res = StatusCode::METHOD_NOT_ALLOWED.into_response();
            if let Ok(v) = HeaderValue::try_from(allow.join(", ")) {
                res.headers_mut().insert(header::ALLOW, v);
            }
            return ResponseFuture::ready(res);
        };

        req.extensions_mut().insert(params);
        req.extensions_mut().insert(BodyLimit(inner.table[*t].body_limit));
        req.extensions_mut().insert(Matched(inner.table[*t].clone()));
        req.extensions_mut().insert(UrlFor(inner.urls.clone()));
        ResponseFuture(Kind::Service(svc.clone().oneshot(req)))
    }
}

impl Service<Request<Body>> for Routes {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = ResponseFuture;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> ResponseFuture {
        self.dispatch(req)
    }
}

/// The router's future. Not boxed: 404 and 405 are ready values, a match is
/// the route's own service.
pub struct ResponseFuture(Kind);

enum Kind {
    Service(Oneshot<BoxService, Request<Body>>),
    Ready(Option<Response<Body>>),
}

impl ResponseFuture {
    fn ready(res: Response<Body>) -> Self {
        Self(Kind::Ready(Some(res)))
    }
}

impl Future for ResponseFuture {
    type Output = Result<Response<Body>, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.get_mut().0 {
            Kind::Service(f) => Pin::new(f).poll(cx),
            Kind::Ready(res) => Poll::Ready(Ok(res.take().expect("ResponseFuture polled after completion"))),
        }
    }
}

#[derive(Debug)]
pub enum BuildError {
    DuplicateName(String),
    DuplicateRoute(Method, String),
    Conflict { path: String, reason: String },
    InvalidPath(String),
    UnregisteredName(&'static str),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName(n) => write!(f, "route name `{n}` is registered twice"),
            Self::DuplicateRoute(m, p) => write!(f, "route {m} {p} is registered twice"),
            Self::Conflict { path, reason } => write!(f, "route {path} conflicts: {reason}"),
            Self::InvalidPath(p) => write!(f, "route path `{p}` must start with `/`"),
            Self::UnregisteredName(n) => write!(f, "route name `{n}` has no route"),
        }
    }
}

impl std::error::Error for BuildError {}
