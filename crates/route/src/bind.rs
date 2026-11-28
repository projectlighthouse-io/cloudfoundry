//! Route model binding: a path parameter resolved to an app model.

use std::{future::Future, ops::Deref, str::FromStr, sync::Arc};

use http::{Request, Response, StatusCode, request::Parts};

use crate::{Body, BoxError, FromRequest, FromRequestParts, IntoResponse, extract::PathParams};

/// A model a path parameter names. Take [`Bound<T>`] in a handler to get it.
///
/// ```ignore
/// pub struct Machine { pub id: i64, pub uuid: Uuid<Machine>, pub name: String }
///
/// impl route::Bind<App> for Machine {
///     const PARAM: &'static str = "machine";
///     type Key = Uuid<Machine>;
///     async fn resolve(key: Uuid<Machine>, app: &App) -> Result<Option<Self>, route::BoxError> {
///         Ok(sqlx::query_as("select id, uuid, name from machines where uuid = $1")
///             .bind(key)
///             .fetch_optional(app.db.pool())
///             .await?)
///     }
/// }
///
/// // r.get("/machines/{machine}", show);
/// async fn show(Bound(machine): Bound<Machine>) -> String { machine.name }
/// ```
///
/// A key that does not parse (`/machines/42` for a `Uuid`) and a key that
/// resolves to `None` are both `404`. Checking who may see the model is the
/// handler's (or `resolve`'s) job.
pub trait Bind<S>: Sized + Send {
    /// The path parameter's name, without braces.
    const PARAM: &'static str;
    type Key: FromStr;
    /// The model for `key`, `None` if there is none. An error is logged and is `500`.
    fn resolve(key: Self::Key, state: &S) -> impl Future<Output = Result<Option<Self>, BoxError>> + Send;
}

/// The model named by `{T::PARAM}` in the path. See [`Bind`].
#[derive(Debug)]
pub struct Bound<T>(pub T);

impl<T> Deref for Bound<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<S: Send + Sync, T: Bind<S>> FromRequestParts<S> for Bound<T> {
    async fn from_request_parts(parts: &mut Parts, state: &Arc<S>) -> Result<Self, Response<Body>> {
        let not_found = || StatusCode::NOT_FOUND.into_response();
        let key = parts
            .extensions
            .get::<PathParams>()
            .and_then(|p| p.0.iter().find(|(k, _)| k == T::PARAM))
            .and_then(|(_, v)| v.parse::<T::Key>().ok())
            .ok_or_else(not_found)?;
        match T::resolve(key, state).await {
            Ok(Some(model)) => Ok(Bound(model)),
            Ok(None) => Err(not_found()),
            Err(e) => {
                tracing::error!("binding `{{{}}}`: {e}", T::PARAM);
                Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
            }
        }
    }
}

impl<S: Send + Sync, T: Bind<S>> FromRequest<S> for Bound<T> {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        crate::from_parts(req, state)
    }
}
