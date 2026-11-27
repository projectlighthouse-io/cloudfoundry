//! Issue and verify CSRF tokens.
//!
//! One token per session. Unsafe methods (`POST`, `PUT`, `PATCH`, `DELETE`)
//! must echo it in `X-XSRF-TOKEN` (what the Inertia v3 client sends, read from
//! the `XSRF-TOKEN` cookie this layer sets), `X-CSRF-TOKEN`, or, for a plain
//! HTML form, a `_token` field. A failure is a bare `419`.
//!
//! ```ignore
//! router
//!     .layer(session::layer(store, codec))        // outer: provides the token store
//!     .layer(csrf::layer::<Session>().except(["webhooks.github"]));
//! ```

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, Request, Response, StatusCode, header};
use tower::{Layer, Service};

/// Where the token lives. `session::Session` implements it (feature `csrf`).
/// The layer finds the store in request extensions.
pub trait TokenStore {
    fn token(&self) -> Option<String>;
    fn set_token(&mut self, token: String);
}

const COOKIE: &str = "XSRF-TOKEN";
const HEADERS: [&str; 2] = ["x-xsrf-token", "x-csrf-token"];

pub struct CsrfLayer<T> {
    except: Arc<Vec<String>>,
    secure: bool,
    _store: PhantomData<fn() -> T>,
}

/// The CSRF layer, reading tokens from the `T` in request extensions.
pub fn layer<T>() -> CsrfLayer<T> {
    CsrfLayer { except: Arc::new(vec![]), secure: true, _store: PhantomData }
}

impl<T> CsrfLayer<T> {
    /// Skip routes by name, e.g. incoming webhooks that authenticate otherwise.
    #[cfg(feature = "route")]
    pub fn except<N: route::Name>(mut self, names: impl IntoIterator<Item = N>) -> Self {
        Arc::make_mut(&mut self.except).extend(names.into_iter().map(|n| n.as_str().to_owned()));
        self
    }

    /// `Secure` on the `XSRF-TOKEN` cookie. Default `true`.
    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }
}

impl<T> Clone for CsrfLayer<T> {
    fn clone(&self) -> Self {
        Self { except: self.except.clone(), secure: self.secure, _store: PhantomData }
    }
}

impl<S, T> Layer<S> for CsrfLayer<T> {
    type Service = Csrf<S, T>;

    fn layer(&self, inner: S) -> Self::Service {
        Csrf { inner, cfg: self.clone() }
    }
}

pub struct Csrf<S, T> {
    inner: S,
    cfg: CsrfLayer<T>,
}

impl<S: Clone, T> Clone for Csrf<S, T> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), cfg: self.cfg.clone() }
    }
}

impl<S, T, ReqB, ResB> Service<Request<ReqB>> for Csrf<S, T>
where
    S: Service<Request<ReqB>, Response = Response<ResB>> + Clone + Send + 'static,
    S::Future: Send,
    T: TokenStore + Clone + Send + Sync + 'static,
    ReqB: FormBody,
    ResB: Default + Send + 'static,
{
    type Response = Response<ResB>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response<ResB>, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqB>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let secure = self.cfg.secure;

        if self.exempt(&req) {
            return Box::pin(inner.call(req));
        }
        let Some(mut store) = req.extensions().get::<T>().cloned() else {
            tracing::error!("no token store in request; add the session layer before the csrf layer");
            return Box::pin(async { Ok(status(StatusCode::INTERNAL_SERVER_ERROR)) });
        };

        // ponytail: issues a token (and so a session) on the first request of every
        // visitor; make it lazy if anonymous traffic makes the session table hot.
        let token = store.token().unwrap_or_else(|| {
            let t = random_token();
            store.set_token(t.clone());
            t
        });

        let unsafe_method = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE);
        let header_ok = HEADERS.iter().any(|h| header_value(req.headers(), h).is_some_and(|v| ct_eq(v, &token)));
        let form = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.split(';').next().is_some_and(|m| m.trim().eq_ignore_ascii_case("application/x-www-form-urlencoded")));
        // 419 is Laravel's "page expired"; Inertia clients know it.
        let expired = || Ok(status(StatusCode::from_u16(419).unwrap_or(StatusCode::FORBIDDEN)));
        if unsafe_method && !header_ok && !form {
            return Box::pin(async move { expired() });
        }

        let sent = cookie(req.headers(), COOKIE).map(str::to_owned);
        Box::pin(async move {
            let req = if unsafe_method && !header_ok {
                // A plain HTML form: the token is the `_token` field. Read the body,
                // check it, and hand the same bytes on.
                let (parts, body) = req.into_parts();
                let Some(bytes) = read_limited(body, form_limit(&parts.extensions)).await else { return expired() };
                let field = form_field(&bytes, "_token");
                if !field.is_some_and(|f| ct_eq(&f, &token)) {
                    return expired();
                }
                Request::from_parts(parts, ReqB::from_bytes(bytes))
            } else {
                req
            };
            let mut res = inner.call(req).await?;
            // The handler may have rotated the token (login and logout regenerate
            // the session). Send the current one, so a client that posts again
            // without a page visit in between still has it.
            let token = store.token().unwrap_or_else(|| {
                let t = random_token();
                store.set_token(t.clone());
                t
            });
            if sent.as_deref() != Some(token.as_str()) {
                let secure = if secure { "; Secure" } else { "" };
                if let Ok(v) = HeaderValue::try_from(format!("{COOKIE}={token}; Path=/; SameSite=Lax{secure}")) {
                    res.headers_mut().append(header::SET_COOKIE, v);
                }
            }
            Ok(res)
        })
    }
}

impl<S, T> Csrf<S, T> {
    #[cfg(feature = "route")]
    fn exempt<B>(&self, req: &Request<B>) -> bool {
        req.extensions()
            .get::<route::Matched>()
            .and_then(|m| m.0.name.as_deref())
            .is_some_and(|n| self.cfg.except.iter().any(|e| e == n))
    }

    #[cfg(not(feature = "route"))]
    #[allow(clippy::unused_self, reason = "same signature as the `route` version")]
    fn exempt<B>(&self, _: &Request<B>) -> bool {
        false
    }
}

/// A request body the layer can read and rebuild, to find `_token` in a form.
/// Implemented for `route::Body` (feature `route`) and `Full<Bytes>`.
pub trait FormBody: http_body::Body<Data = Bytes> + Send + Unpin + Sized + 'static {
    fn from_bytes(bytes: Bytes) -> Self;
}

impl FormBody for http_body_util::Full<Bytes> {
    fn from_bytes(bytes: Bytes) -> Self {
        http_body_util::Full::new(bytes)
    }
}

#[cfg(feature = "route")]
impl FormBody for route::Body {
    fn from_bytes(bytes: Bytes) -> Self {
        route::body(bytes)
    }
}

/// Largest form body read for `_token` without `route`'s per-route limit.
const FORM_LIMIT: usize = 2 * 1024 * 1024;

#[cfg(feature = "route")]
fn form_limit(ext: &http::Extensions) -> usize {
    ext.get::<route::BodyLimit>().map_or(FORM_LIMIT, |l| l.0)
}

#[cfg(not(feature = "route"))]
fn form_limit(_: &http::Extensions) -> usize {
    FORM_LIMIT
}

async fn read_limited<B: FormBody>(mut body: B, limit: usize) -> Option<Bytes> {
    use http_body_util::BodyExt;
    let mut buf = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.ok()?.into_data() {
            if buf.len() + data.len() > limit {
                return None;
            }
            buf.extend_from_slice(&data);
        }
    }
    Some(buf.into())
}

fn form_field(body: &[u8], name: &str) -> Option<String> {
    let pairs: Vec<(String, String)> = serde_urlencoded::from_bytes(body).ok()?;
    pairs.into_iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

fn status<B: Default>(s: StatusCode) -> Response<B> {
    let mut res = Response::new(B::default());
    *res.status_mut() = s;
    res
}

fn random_token() -> String {
    let mut b = [0u8; 32];
    // No secure fallback exists: a guessable CSRF token must not be issued.
    #[allow(clippy::expect_used, reason = "the OS random source failed; there is no safe fallback")]
    getrandom::fill(&mut b).expect("OS random source");
    URL_SAFE_NO_PAD.encode(b)
}

/// Constant time for equal lengths. Token length is public, so an early
/// return on a length mismatch leaks nothing.
fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
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
