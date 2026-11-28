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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "configured",
    derive(configured::serde::Deserialize, configured::serde::Serialize),
    serde(crate = "configured::serde", rename_all = "lowercase")
)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

/// Cookie settings. Deserializable with feature `configured`; missing keys keep
/// their defaults, unknown keys are an error:
///
/// ```toml
/// [session]
/// cookie_name = "app_session"
/// lifetime = "2h"
/// same_site = "strict"   # strict | lax | none
/// secure = true
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "configured",
    derive(configured::serde::Deserialize, configured::serde::Serialize),
    serde(crate = "configured::serde", default, deny_unknown_fields)
)]
pub struct Config {
    pub cookie_name: String,
    #[cfg_attr(feature = "configured", serde(with = "configured::duration"))]
    pub lifetime: Duration,
    pub same_site: SameSite,
    /// Browsers treat `localhost` as secure, so `true` works in local dev too.
    pub secure: bool,
}

/// Cookie `session`, 14 days, `SameSite=Lax`, `Secure`.
impl Default for Config {
    fn default() -> Self {
        Self { cookie_name: "session".into(), lifetime: Duration::from_secs(14 * 86400), same_site: SameSite::Lax, secure: true }
    }
}

/// A [`Config`] a browser would reject or that breaks every session.
#[derive(Debug)]
pub struct ConfigError(String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Run by [`SessionLayer::config`]. Public for apps that build the layer later.
    pub fn check(&self) -> Result<(), ConfigError> {
        let err = |m: String| Err(ConfigError(m));
        let token = |c: u8| c.is_ascii_graphic() && !b"()<>@,;:\\\"/[]?={}".contains(&c);
        if self.cookie_name.is_empty() || !self.cookie_name.bytes().all(token) {
            return err(format!("cookie name `{}` is not a valid cookie token", self.cookie_name));
        }
        if self.lifetime.as_secs() == 0 {
            return err("lifetime must be at least one second".into());
        }
        if self.same_site == SameSite::None && !self.secure {
            return err("SameSite=None requires secure = true; browsers drop the cookie otherwise".into());
        }
        Ok(())
    }
}

/// Loads the session before the handler, saves it after if it changed.
pub struct SessionLayer<St> {
    store: Arc<St>,
    codec: Arc<Codec>,
    cfg: Config,
}

/// `session::layer(store, codec)` with [`Config::default`].
pub fn layer<St: Store>(store: St, codec: Codec) -> SessionLayer<St> {
    SessionLayer { store: Arc::new(store), codec: Arc::new(codec), cfg: Config::default() }
}

impl<St> SessionLayer<St> {
    /// Replace all settings after [`Config::check`]. Setters called after
    /// this change single fields and are not checked.
    pub fn config(mut self, cfg: Config) -> Result<Self, ConfigError> {
        cfg.check()?;
        self.cfg = cfg;
        Ok(self)
    }

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
            shared: Arc::new(Shared { store: self.store.clone(), codec: self.codec.clone(), cfg: self.cfg.clone() }),
        }
    }
}

struct Shared<St> {
    store: Arc<St>,
    codec: Arc<Codec>,
    cfg: Config,
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
            let lifetime = sh.cfg.lifetime.as_secs() as i64;

            let id = cookie(req.headers(), &sh.cfg.cookie_name).and_then(|v| sh.codec.decrypt(&sh.cfg.cookie_name, v, sh.cfg.lifetime).ok());
            let session = match id {
                Some(id) => match sh.store.load(&id).await {
                    Ok(Some(rec)) if rec.expires_at > now => Session::load(rec),
                    Ok(_) => Session::empty(),
                    Err(e) => return Ok(failure("load", e)),
                },
                None => Session::empty(),
            };

            let user_agent = req.headers().get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(|v| v.chars().take(crate::MAX_USER_AGENT).collect());
            #[cfg(feature = "route")]
            let ip = req.extensions().get::<route::ClientAddr>().map(|a| a.0.ip().to_string());
            #[cfg(not(feature = "route"))]
            let ip = None;
            session.seen(user_agent, ip);
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
                match sh.codec.encrypt(&sh.cfg.cookie_name, &rec.id) {
                    Ok(v) => set_cookie(&mut res, &sh, &v, sh.cfg.lifetime.as_secs()),
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
    let same_site = match sh.cfg.same_site {
        SameSite::Strict => "Strict",
        SameSite::Lax => "Lax",
        SameSite::None => "None",
    };
    let secure = if sh.cfg.secure { "; Secure" } else { "" };
    let c = format!("{}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite={same_site}{secure}", sh.cfg.cookie_name);
    if let Ok(v) = HeaderValue::try_from(c) {
        res.headers_mut().append(header::SET_COOKIE, v);
    } else {
        tracing::error!("session cookie `{}` is not a valid header value; cookie not set", sh.cfg.cookie_name);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_rejects_what_browsers_drop() {
        assert!(Config::default().check().is_ok());
        for name in ["", "a b", "a;b", "a=b", "sé"] {
            assert!(Config { cookie_name: name.into(), ..Config::default() }.check().is_err(), "{name:?}");
        }
        assert!(Config { lifetime: Duration::from_millis(500), ..Config::default() }.check().is_err());
        assert!(Config { same_site: SameSite::None, secure: false, ..Config::default() }.check().is_err());
        assert!(Config { same_site: SameSite::None, secure: true, ..Config::default() }.check().is_ok());
    }
}
