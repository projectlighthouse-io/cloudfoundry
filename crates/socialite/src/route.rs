//! Ready-made routes for a provider (feature `route`).
//!
//! ```ignore
//! socialite::routes(&mut r, google, handlers::social::callback);
//! // GET /auth/google           -> redirect to Google (route name `social.google`)
//! // GET /auth/google/callback  -> your handler      (route name `social.google.callback`)
//!
//! async fn callback(cx: Cx<App>, (auth, social): (Auth<User>, SocialCallback)) -> Redirect {
//!     match social.0 {
//!         Ok(user) => { /* find or create, auth.login */ }
//!         Err(e) => { /* declined, forged state, ... */ }
//!     }
//! }
//! ```

use std::{future::Future, pin::Pin, sync::Arc};

use route::{
    Body, Cx, FromRequest, FromRequestParts, Handler, IntoResponse, Redirect, Router,
    http::{Request, Response, StatusCode, request::Parts},
};
use session::Session;

use crate::{Callback, Error, Provider, SocialUser};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A provider with its types erased, so one input type serves every provider.
trait Erased: Send + Sync {
    fn authorize_url(&self, session: &Session) -> String;
    fn social_user<'a>(&'a self, session: &'a Session, cb: Callback) -> BoxFuture<'a, Result<SocialUser, Error>>;
}

impl<P: Provider> Erased for P {
    fn authorize_url(&self, session: &Session) -> String {
        Provider::authorize_url(self, session)
    }

    fn social_user<'a>(&'a self, session: &'a Session, cb: Callback) -> BoxFuture<'a, Result<SocialUser, Error>> {
        Box::pin(Provider::social_user(self, session, cb))
    }
}

/// The provider whose callback is running; in request extensions.
#[derive(Clone)]
struct Current(Arc<dyn Erased>);

/// Register `GET /auth/{name}` (redirect to the provider) and
/// `GET /auth/{name}/callback` (your `callback` handler, which takes a
/// [`SocialCallback`]). Route names: `social.{name}` and `social.{name}.callback`.
/// Needs the session layer.
pub fn routes<S, P, H, M>(r: &mut Router<S>, provider: P, callback: H)
where
    S: Send + Sync + 'static,
    P: Provider + 'static,
    H: Handler<S, M>,
    M: 'static,
{
    let name = provider.name();
    let provider: Arc<dyn Erased> = Arc::new(provider);
    // ponytail: route names must be 'static; one small leak per provider at startup.
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };

    let p = provider.clone();
    r.get(&format!("/auth/{name}"), move |session: Session| {
        let p = p.clone();
        async move { Redirect::to(p.authorize_url(&session)) }
    })
    .name(leak(format!("social.{name}")));

    r.get(&format!("/auth/{name}/callback"), move |(cx, mut req): (Cx<S>, Request<Body>)| {
        let (callback, p) = (callback.clone(), provider.clone());
        async move {
            req.extensions_mut().insert(Current(p));
            callback.call(req, cx.0).await
        }
    })
    .name(leak(format!("social.{name}.callback")));
}

/// The result of a provider's callback, already checked and exchanged: the
/// user, or why there is none (declined, forged `state`, network failure).
/// Only on routes registered by [`routes`].
pub struct SocialCallback(pub Result<SocialUser, Error>);

impl<S: Send + Sync> FromRequestParts<S> for SocialCallback {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        let server_error = |msg: &str| (StatusCode::INTERNAL_SERVER_ERROR, msg.to_owned()).into_response();
        let Some(Current(provider)) = parts.extensions.get::<Current>().cloned() else {
            return Err(server_error("SocialCallback outside a socialite::routes callback"));
        };
        let Some(session) = parts.extensions.get::<Session>().cloned() else {
            return Err(server_error("socialite needs the session layer"));
        };
        let cb: Callback = serde_urlencoded::from_str(parts.uri.query().unwrap_or_default())
            .unwrap_or(Callback { code: None, state: None, error: Some("malformed callback".into()) });
        Ok(SocialCallback(provider.social_user(&session, cb).await))
    }
}

impl<S: Send + Sync> FromRequest<S> for SocialCallback {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}
