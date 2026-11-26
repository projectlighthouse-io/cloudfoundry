//! `multipart/form-data` bodies, parsed as a stream by `multer`.

use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

use http::{Request, Response, StatusCode, header};
use http_body_util::BodyExt;

use crate::{Body, FromRequest, IntoResponse, extract::limit_of};

pub use multer::{Error as MultipartError, Field};

/// A `multipart/form-data` body. Read it field by field:
///
/// ```ignore
/// async fn upload(mut form: Multipart) -> Result<String, MultipartError> {
///     while let Some(field) = form.next_field().await? {      // over the limit: 413
///         let name = field.name().unwrap_or("").to_owned();
///         let bytes = field.bytes().await?;
///     }
///     Ok("ok".into())
/// }
/// ```
///
/// The whole body is capped at the route's body limit; reading past it is an
/// error from `next_field` or the field's read.
pub struct Multipart(multer::Multipart<'static>);

impl Deref for Multipart {
    type Target = multer::Multipart<'static>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Multipart {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<S: Send + Sync> FromRequest<S> for Multipart {
    async fn from_request(req: Request<Body>, _: &Arc<S>) -> Result<Self, Response<Body>> {
        let boundary = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|ct| multer::parse_boundary(ct).ok())
            .ok_or_else(|| StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response())?;
        let limit = limit_of(&req) as u64;
        let constraints = multer::Constraints::new().size_limit(multer::SizeLimit::new().whole_stream(limit));
        let stream = req.into_body().into_data_stream();
        Ok(Multipart(multer::Multipart::with_constraints(stream, boundary, constraints)))
    }
}

/// Over the body limit is a `413`; any other malformed body a `400`.
impl IntoResponse for MultipartError {
    fn into_response(self) -> Response<Body> {
        match self {
            MultipartError::StreamSizeExceeded { .. } | MultipartError::FieldSizeExceeded { .. } => {
                StatusCode::PAYLOAD_TOO_LARGE.into_response()
            }
            e => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        }
    }
}
