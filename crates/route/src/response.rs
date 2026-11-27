use bytes::Bytes;
use http::{HeaderValue, Response, StatusCode, header};
use serde::Serialize;

use crate::{Body, Json, Name, body, empty};

/// A handler output.
pub trait IntoResponse {
    fn into_response(self) -> Response<Body>;
}

impl IntoResponse for Response<Body> {
    fn into_response(self) -> Response<Body> {
        self
    }
}

impl IntoResponse for () {
    fn into_response(self) -> Response<Body> {
        Response::new(empty())
    }
}

impl IntoResponse for StatusCode {
    fn into_response(self) -> Response<Body> {
        let mut res = Response::new(empty());
        *res.status_mut() = self;
        res
    }
}

fn with_type(b: impl Into<Bytes>, content_type: &'static str) -> Response<Body> {
    let mut res = Response::new(body(b));
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    res
}

impl IntoResponse for &'static str {
    fn into_response(self) -> Response<Body> {
        with_type(self, "text/plain; charset=utf-8")
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Response<Body> {
        with_type(self, "text/plain; charset=utf-8")
    }
}

impl IntoResponse for Bytes {
    fn into_response(self) -> Response<Body> {
        with_type(self, "application/octet-stream")
    }
}

/// An HTML response: `text/html; charset=utf-8`. A plain `String` is sent as
/// `text/plain`, so browsers would show the markup instead of rendering it.
///
/// ```
/// async fn hello(_: ()) -> route::Html<String> {
///     route::Html(format!("<h1>Hello, {}</h1>", "Ada"))
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Html<T>(pub T);

impl<T: Into<Bytes>> IntoResponse for Html<T> {
    fn into_response(self) -> Response<Body> {
        with_type(self.0, "text/html; charset=utf-8")
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response<Body> {
        match serde_json::to_vec(&self.0) {
            Ok(v) => with_type(v, "application/json"),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}

impl<T: IntoResponse> IntoResponse for (StatusCode, T) {
    fn into_response(self) -> Response<Body> {
        let mut res = self.1.into_response();
        *res.status_mut() = self.0;
        res
    }
}

impl<T: IntoResponse, E: IntoResponse> IntoResponse for Result<T, E> {
    fn into_response(self) -> Response<Body> {
        match self {
            Ok(v) => v.into_response(),
            Err(e) => e.into_response(),
        }
    }
}

#[derive(Clone)]
/// Route name and params waiting for the router to turn them into a URL.
pub(crate) struct PendingRoute(pub(crate) &'static str, pub(crate) Vec<(String, String)>);

/// A `303 See Other` redirect, to a URL or a named route.
pub struct Redirect {
    target: Target,
}

enum Target {
    Url(String),
    Route(&'static str, Vec<(String, String)>),
}

impl Redirect {
    pub fn to(url: impl Into<String>) -> Self {
        Self { target: Target::Url(url.into()) }
    }

    /// Resolved by the router. An unknown name or a wrong parameter becomes a 500.
    pub fn to_route(name: impl Name) -> Self {
        Self { target: Target::Route(name.as_str(), vec![]) }
    }

    /// Path parameters for `to_route`. Ignored by `to`.
    pub fn params(mut self, params: &[(&str, &str)]) -> Self {
        if let Target::Route(_, p) = &mut self.target {
            p.extend(params.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        }
        self
    }
}

impl IntoResponse for Redirect {
    fn into_response(self) -> Response<Body> {
        let mut res = StatusCode::SEE_OTHER.into_response();
        match self.target {
            Target::Url(url) => match HeaderValue::try_from(url) {
                Ok(v) => {
                    res.headers_mut().insert(header::LOCATION, v);
                }
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            },
            Target::Route(name, params) => {
                res.extensions_mut().insert(PendingRoute(name, params));
            }
        }
        res
    }
}
