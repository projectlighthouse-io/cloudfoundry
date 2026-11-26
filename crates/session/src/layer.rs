use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use http::{HeaderMap, HeaderValue, Request, Response, StatusCode, header};
use securecookie::Codec;
use tower::{Layer, Service};

use crate::{Session, Store, unix_now};

#[derive(Clone, Copy, Debug)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

struct Config {
    cookie_name: String,
    lifetime: Duration,
    same_site: SameSite,
    secure: bool,
}

/// Loads the session before the handler, saves it after if it changed.
pub struct SessionLayer<St> {
    store: Arc<St>,
    codec: Arc<Codec>,
    cfg: Config,
}

/// `session::layer(store, codec)`. Defaults: cookie `session`, 14 days,
/// `SameSite=Lax`, `Secure`.
pub fn layer<St: Store>(store: St, codec: Codec) -> SessionLayer<St> {
    SessionLayer {
        store: Arc::new(store),
        codec: Arc::new(codec),
        cfg: Config {
            cookie_name: "session".into(),
            lifetime: Duration::from_secs(14 * 86400),
            same_site: SameSite::Lax,
            secure: true,
        },
    }
}

impl<St> SessionLayer<St> {
    pub fn cookie_name(mut self, name: &str) -> Self {
        self.cfg.cookie_name = name.to_owned();
        self
    }

    pub fn lifetime(mut self, lifetime: Duration) -> Self {
        self.cfg.lifetime = lifetime;
        self
    }

    pub fn same_site(mut self, same_site: SameSite) -> Self {
        self.cfg.same_site = same_site;
        self
    }

    /// Browsers treat `localhost` as secure, so `true` works in local dev too.
    pub fn secure(mut self, secure: bool) -> Self {
        self.cfg.secure = secure;
        self
    }
}

impl<S, St> Layer<S> for SessionLayer<St> {
    type Service = SessionService<S, St>;

    fn layer(&self, inner: S) -> Self::Service {
        SessionService {
            inner,
            shared: Arc::new(Shared {
                store: self.store.clone(),
                codec: self.codec.clone(),
                cookie_name: self.cfg.cookie_name.clone(),
                lifetime: self.cfg.lifetime,
                same_site: self.cfg.same_site,
                secure: self.cfg.secure,
            }),
        }
    }
}

struct Shared<St> {
    store: Arc<St>,
    codec: Arc<Codec>,
    cookie_name: String,
    lifetime: Duration,
    same_site: SameSite,
    secure: bool,
}

pub struct SessionService<S, St> {
    inner: S,
    shared: Arc<Shared<St>>,
}

impl<S: Clone, St> Clone for SessionService<S, St> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), shared: self.shared.clone() }
    }
}

impl<S, St, ReqB, ResB> Service<Request<ReqB>> for SessionService<S, St>
where
    S: Service<Request<ReqB>, Response = Response<ResB>> + Clone + Send + 'static,
    S::Future: Send,
    St: Store,
    ReqB: Send + 'static,
    ResB: Default + Send + 'static,
{
    type Response = Response<ResB>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response<ResB>, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqB>) -> Self::Future {
        // The ready service handles this request; a fresh clone takes its place.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let sh = self.shared.clone();

        Box::pin(async move {
            let now = unix_now();
            let lifetime = sh.lifetime.as_secs() as i64;

            let id = cookie(req.headers(), &sh.cookie_name).and_then(|v| sh.codec.decrypt(&sh.cookie_name, v, sh.lifetime).ok());
            let session = match id {
                Some(id) => match sh.store.load(&id).await {
                    Ok(Some(rec)) if rec.expires_at > now => Session::load(rec),
                    Ok(_) => Session::empty(),
                    Err(e) => return Ok(failure("load", e)),
                },
                None => Session::empty(),
            };

            req.extensions_mut().insert(session.clone());
            let mut res = inner.call(req).await?;

            let plan = session.finish(now, lifetime);
            for id in &plan.delete {
                if let Err(e) = sh.store.delete(id).await {
                    return Ok(failure("delete", e));
                }
            }
            if let Some(rec) = &plan.save {
                if let Err(e) = sh.store.save(rec).await {
                    return Ok(failure("save", e));
                }
                match sh.codec.encrypt(&sh.cookie_name, &rec.id) {
                    Ok(v) => set_cookie(&mut res, &sh, &v, sh.lifetime.as_secs()),
                    Err(_) => return Ok(failure("encrypt", "cookie encryption failed".into())),
                }
            } else if plan.expire_cookie {
                set_cookie(&mut res, &sh, "", 0);
            }
            Ok(res)
        })
    }
}

/// A store failure fails the request: dropping a write silently loses data.
fn failure<B: Default>(op: &str, e: crate::Error) -> Response<B> {
    tracing::error!("session {op} failed: {e}");
    let mut res = Response::new(B::default());
    *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    res
}

fn set_cookie<B, St>(res: &mut Response<B>, sh: &Shared<St>, value: &str, max_age: u64) {
    let same_site = match sh.same_site {
        SameSite::Strict => "Strict",
        SameSite::Lax => "Lax",
        SameSite::None => "None",
    };
    let secure = if sh.secure { "; Secure" } else { "" };
    let c = format!("{}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite={same_site}{secure}", sh.cookie_name);
    if let Ok(v) = HeaderValue::try_from(c) {
        res.headers_mut().append(header::SET_COOKIE, v);
    }
}

pub(crate) fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}
