use std::{future::Future, ops::Deref, sync::Arc};

use bytes::Bytes;
use http::{Request, Response, StatusCode, header, request::Parts};
use http_body_util::BodyExt;
use percent_encoding::percent_decode_str;
use serde::de::DeserializeOwned;

use crate::{Body, IntoResponse};

/// Largest body `Form`, `Json`, `Bytes` and `Request<Bytes>` read when no
/// limit is set with `Router::body_limit` or `Route::body_limit`.
pub const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// The body limit for the matched route, in request extensions. Set by the
/// router; layers that read bodies (CSRF's `_token`) honour it too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyLimit(pub usize);

pub(crate) fn limit_of<B>(req: &Request<B>) -> usize {
    req.extensions().get::<BodyLimit>().map_or(DEFAULT_BODY_LIMIT, |l| l.0)
}

/// A handler input, parsed from the request. A failure is the response.
pub trait FromRequest<S>: Sized + Send {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send;
}

/// An input read from the request head only (no body). Every such input can
/// sit before the last element of a tuple input: `(Session, Valid<Form<Login>>)`.
pub trait FromRequestParts<S>: Sized + Send {
    fn from_request_parts(parts: &mut Parts, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send;
}

/// `FromRequest` for a `FromRequestParts` type: drop the body, read the head.
/// Use it to implement both traits for one input.
pub async fn from_parts<S, T: FromRequestParts<S>>(req: Request<Body>, state: &Arc<S>) -> Result<T, Response<Body>> {
    let (mut parts, _) = req.into_parts();
    T::from_request_parts(&mut parts, state).await
}

macro_rules! via_parts {
    ($([$($g:tt)*] $ty:ty),* $(,)?) => {$(
        impl<S: Send + Sync, $($g)*> FromRequest<S> for $ty {
            fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
                from_parts(req, state)
            }
        }
    )*};
}

via_parts!([] (), [] Cx<S>, [] crate::ClientAddr, [T: DeserializeOwned + Send] Path<T>, [T: DeserializeOwned + Send] Query<T>);

macro_rules! tuple {
    ($($p:ident),* ; $last:ident) => {
        #[allow(non_snake_case)]
        impl<S: Send + Sync, $($p: FromRequestParts<S>,)* $last: FromRequest<S>> FromRequest<S> for ($($p,)* $last,) {
            async fn from_request(req: Request<Body>, state: &Arc<S>) -> Result<Self, Response<Body>> {
                let (mut parts, body) = req.into_parts();
                $(let $p = $p::from_request_parts(&mut parts, state).await?;)*
                let $last = $last::from_request(Request::from_parts(parts, body), state).await?;
                Ok(($($p,)* $last,))
            }
        }
    };
}

tuple!(A; B);
tuple!(A, B; C);
tuple!(A, B, C; D);

fn reject(status: StatusCode, msg: impl Into<String>) -> Response<Body> {
    (status, msg.into()).into_response()
}

async fn collect(mut body: Body, limit: usize) -> Result<Bytes, Response<Body>> {
    let mut buf = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| reject(StatusCode::BAD_REQUEST, "failed to read body"))?;
        if let Ok(data) = frame.into_data() {
            if buf.len() + data.len() > limit {
                return Err(reject(StatusCode::PAYLOAD_TOO_LARGE, "payload too large"));
            }
            buf.extend_from_slice(&data);
        }
    }
    Ok(buf.into())
}

fn content_type_is(req: &Request<Body>, mime: &str) -> bool {
    req.headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case(mime))
}

macro_rules! deref {
    ($($wrapper:ident),*) => {$(
        impl<T> Deref for $wrapper<T> {
            type Target = T;
            fn deref(&self) -> &T {
                &self.0
            }
        }
    )*};
}

deref!(Path, Query, Form, Json);

impl<S: Send + Sync> FromRequestParts<S> for () {
    async fn from_request_parts(_: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        Ok(())
    }
}

/// `500` when the request did not come through `serve` (no address recorded).
impl<S: Send + Sync> FromRequestParts<S> for crate::ClientAddr {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        parts.extensions.get::<crate::ClientAddr>().copied().ok_or_else(|| {
            tracing::error!("no client address; requests need to come through `route::serve`");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
    }
}

/// App state. Derefs to `S`.
pub struct Cx<S>(pub Arc<S>);

impl<S> Clone for Cx<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S> Deref for Cx<S> {
    type Target = S;
    fn deref(&self) -> &S {
        &self.0
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Cx<S> {
    async fn from_request_parts(_: &mut Parts, state: &Arc<S>) -> Result<Self, Response<Body>> {
        Ok(Cx(state.clone()))
    }
}

/// Matched path parameters, percent-decoded. Set by the router.
#[derive(Clone, Debug)]
pub(crate) struct PathParams(pub(crate) Vec<(String, String)>);

impl PathParams {
    pub(crate) fn decode<'a>(raw: impl Iterator<Item = (&'a str, &'a str)>) -> Self {
        Self(raw.map(|(k, v)| (k.to_owned(), percent_decode_str(v).decode_utf8_lossy().into_owned())).collect())
    }
}

/// Typed path parameters. One parameter deserializes into a value
/// (`Path<RunId>`), several into a struct.
#[derive(Debug)]
pub struct Path<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for Path<T> {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        let not_found = || StatusCode::NOT_FOUND.into_response();
        let params = parts.extensions.get::<PathParams>().ok_or_else(not_found)?;
        // Round-trip through urlencoded: it parses numbers and newtypes from strings.
        let encoded = serde_urlencoded::to_string(&params.0).map_err(|_| not_found())?;
        if let Ok(v) = serde_urlencoded::from_str::<T>(&encoded) {
            return Ok(Path(v));
        }
        match serde_urlencoded::from_str::<Vec<(String, T)>>(&encoded) {
            Ok(mut v) if v.len() == 1 => Ok(Path(v.pop().unwrap().1)),
            _ => Err(not_found()),
        }
    }
}

#[derive(Debug)]
pub struct Query<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for Query<T> {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        serde_urlencoded::from_str(parts.uri.query().unwrap_or(""))
            .map(Query)
            .map_err(|e| reject(StatusCode::BAD_REQUEST, e.to_string()))
    }
}

/// `application/x-www-form-urlencoded` body.
#[derive(Debug)]
pub struct Form<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequest<S> for Form<T> {
    async fn from_request(req: Request<Body>, _: &Arc<S>) -> Result<Self, Response<Body>> {
        if !content_type_is(&req, "application/x-www-form-urlencoded") {
            return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response());
        }
        let limit = limit_of(&req);
        let bytes = collect(req.into_body(), limit).await?;
        serde_urlencoded::from_bytes(&bytes)
            .map(Form)
            .map_err(|e| reject(StatusCode::BAD_REQUEST, e.to_string()))
    }
}

/// `application/json` body as input, JSON as output.
#[derive(Debug)]
pub struct Json<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequest<S> for Json<T> {
    async fn from_request(req: Request<Body>, _: &Arc<S>) -> Result<Self, Response<Body>> {
        if !content_type_is(&req, "application/json") {
            return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response());
        }
        let limit = limit_of(&req);
        let bytes = collect(req.into_body(), limit).await?;
        serde_json::from_slice(&bytes)
            .map(Json)
            .map_err(|e| reject(StatusCode::BAD_REQUEST, e.to_string()))
    }
}

impl<S: Send + Sync> FromRequest<S> for Bytes {
    async fn from_request(req: Request<Body>, _: &Arc<S>) -> Result<Self, Response<Body>> {
        let limit = limit_of(&req);
        collect(req.into_body(), limit).await
    }
}

/// Headers plus the raw body, read up to the route's body limit. For signed payloads
/// such as webhooks, where the exact bytes matter.
impl<S: Send + Sync> FromRequest<S> for Request<Bytes> {
    async fn from_request(req: Request<Body>, _: &Arc<S>) -> Result<Self, Response<Body>> {
        let limit = limit_of(&req);
        let (parts, body) = req.into_parts();
        Ok(Request::from_parts(parts, collect(body, limit).await?))
    }
}

/// The request as it arrived. Reading the body is up to the handler.
impl<S: Send + Sync> FromRequest<S> for Request<Body> {
    async fn from_request(req: Request<Body>, _: &Arc<S>) -> Result<Self, Response<Body>> {
        Ok(req)
    }
}
