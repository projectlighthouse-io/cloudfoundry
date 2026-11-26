//! Issue and verify CSRF tokens.
//!
//! One token per session. Unsafe methods (`POST`, `PUT`, `PATCH`, `DELETE`)
//! must echo it in `X-XSRF-TOKEN` (what the Inertia v3 client sends, read from
//! the `XSRF-TOKEN` cookie this layer sets) or `X-CSRF-TOKEN`. A failure is a
//! bare `419`.
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
    ReqB: Send + 'static,
    ResB: Default,
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
        if unsafe_method && !HEADERS.iter().any(|h| header_value(req.headers(), h).is_some_and(|v| ct_eq(v, &token))) {
            // 419 is Laravel's "page expired"; Inertia clients know it.
            return Box::pin(async { Ok(status(StatusCode::from_u16(419).expect("valid status"))) });
        }

        let needs_cookie = cookie(req.headers(), COOKIE) != Some(token.as_str());
        Box::pin(async move {
            let mut res = inner.call(req).await?;
            if needs_cookie {
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
    fn exempt<B>(&self, _: &Request<B>) -> bool {
        false
    }
}

fn status<B: Default>(s: StatusCode) -> Response<B> {
    let mut res = Response::new(B::default());
    *res.status_mut() = s;
    res
}

fn random_token() -> String {
    let mut b = [0u8; 32];
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
