use std::{future::Future, sync::Arc};

use action::Action;
use http::{Request, Response};

use crate::{Body, FromRequest, Handler, IntoResponse};

/// Turn an `Action` into a handler. Its `Input` is the handler input, its
/// `Output` the response.
pub fn action<A>(a: A) -> ActionHandler<A> {
    ActionHandler(Arc::new(a))
}

pub struct ActionHandler<A>(Arc<A>);

impl<A> Clone for ActionHandler<A> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[doc(hidden)]
pub struct ActionMarker;

impl<S, A> Handler<S, ActionMarker> for ActionHandler<A>
where
    S: Send + Sync + 'static,
    A: Action + Send + Sync + 'static,
    A::Input: FromRequest<S>,
    A::Output: IntoResponse,
{
    fn call(&self, req: Request<Body>, state: Arc<S>) -> impl Future<Output = Response<Body>> + Send {
        async move {
            match A::Input::from_request(req, &state).await {
                Ok(input) => self.0.run(input).await.into_response(),
                Err(rejection) => rejection,
            }
        }
    }
}
