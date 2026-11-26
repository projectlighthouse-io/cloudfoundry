//! `Valid<Form<T>>` and friends as handler inputs.

use std::sync::Arc;

use route::{
    Body, Form, FromRequest, IntoResponse, Json, Path, Query,
    http::{Request, Response, StatusCode},
};
use serde::Serialize;

use crate::{Errors, Valid, Validate};

macro_rules! delegate {
    ($($wrapper:ident),*) => {$(
        impl<T: Validate> Validate for $wrapper<T> {
            fn validate(&self, e: &mut Errors) {
                self.0.validate(e)
            }
        }
    )*};
}

delegate!(Form, Json, Query, Path);

#[derive(Serialize)]
struct Body422<'a> {
    message: &'static str,
    errors: &'a Errors,
}

/// 422 with `{"message", "errors"}`. The `Errors` also ride in the response
/// extensions, so a layer (such as Inertia's) can turn them into a redirect back.
impl IntoResponse for Errors {
    fn into_response(self) -> Response<Body> {
        let mut res = (StatusCode::UNPROCESSABLE_ENTITY, Json(Body422 { message: "The given data was invalid.", errors: &self }))
            .into_response();
        res.extensions_mut().insert(self);
        res
    }
}

impl<S, T> FromRequest<S> for Valid<T>
where
    S: Send + Sync,
    T: FromRequest<S> + Validate,
{
    async fn from_request(req: Request<Body>, state: &Arc<S>) -> Result<Self, Response<Body>> {
        let value = T::from_request(req, state).await?;
        Valid::check(value).map_err(IntoResponse::into_response)
    }
}
