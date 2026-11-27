use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use route::{
    Body, ClientAddr, IntoResponse,
    http::{HeaderValue, Request, Response, StatusCode, header, request::Parts},
};
use tower::{Layer, Service};

use crate::{Algorithm, Limiter, Store};

/// Throttle every route it wraps (the middleware form): each request counts
/// one hit for `name:key(request)`; over the limit, `429 Too Many Requests`
/// with `Retry-After`. `name` keeps separate layers sharing a store apart.
///
/// ```ignore
/// let limiter = Limiter::memory(SlidingWindow::new(60, Duration::from_secs(60)));
/// r.layer(throttle::layer("api", limiter, throttle::by_ip));
/// ```
///
/// A request for which `key` returns `None` is not counted.
pub fn layer<A, St, K>(name: &'static str, limiter: Limiter<A, St>, key: K) -> Throttle<A, St, K>
where
    A: Algorithm,
    St: Store,
    K: Fn(&Parts) -> Option<String> + Clone + Send + Sync + 'static,
{
    Throttle { name, limiter, key }
}

/// Key by client IP (from `route::serve`). Behind a proxy every request has the
/// proxy's IP; key by a header the proxy sets instead.
pub fn by_ip(parts: &Parts) -> Option<String> {
    let addr = parts.extensions.get::<ClientAddr>();
    if addr.is_none() {
        tracing::warn!("throttle::by_ip: no client address; request not counted");
    }
    addr.map(|a| a.0.ip().to_string())
}

pub struct Throttle<A, St, K> {
    name: &'static str,
    limiter: Limiter<A, St>,
    key: K,
}

impl<A, St, K: Clone> Clone for Throttle<A, St, K> {
    fn clone(&self) -> Self {
        Self { name: self.name, limiter: self.limiter.clone(), key: self.key.clone() }
    }
}

impl<S, A, St, K: Clone> Layer<S> for Throttle<A, St, K> {
    type Service = ThrottleService<S, A, St, K>;

    fn layer(&self, inner: S) -> Self::Service {
        ThrottleService { inner, throttle: self.clone() }
    }
}

pub struct ThrottleService<S, A, St, K> {
    inner: S,
    throttle: Throttle<A, St, K>,
}

impl<S: Clone, A, St, K: Clone> Clone for ThrottleService<S, A, St, K> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), throttle: self.throttle.clone() }
    }
}

impl<S, A, St, K, ReqB> Service<Request<ReqB>> for ThrottleService<S, A, St, K>
where
    S: Service<Request<ReqB>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send,
    A: Algorithm,
    St: Store,
    K: Fn(&Parts) -> Option<String> + Clone + Send + Sync + 'static,
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
        let t = self.throttle.clone();
        let (parts, body) = req.into_parts();
        let key = (t.key)(&parts).map(|k| format!("{}:{k}", t.name));
        let req = Request::from_parts(parts, body);
        Box::pin(async move {
            let Some(key) = key else { return inner.call(req).await };
            match t.limiter.hit(&key).await {
                Ok(d) if d.allowed => inner.call(req).await,
                Ok(d) => {
                    let mut res = StatusCode::TOO_MANY_REQUESTS.into_response();
                    let secs = d.retry_after.as_millis().div_ceil(1000).min(u64::MAX as u128) as u64;
                    res.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(secs));
                    Ok(res)
                }
                Err(e) => {
                    tracing::error!("throttle store failed: {e}");
                    Ok(StatusCode::INTERNAL_SERVER_ERROR.into_response())
                }
            }
        })
    }
}
