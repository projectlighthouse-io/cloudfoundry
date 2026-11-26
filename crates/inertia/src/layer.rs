use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use route::{
    Body, IntoResponse, body,
    http::{Extensions, HeaderValue, Method, Request, Response, StatusCode, header},
};
use serde::Serialize;
use serde_json::value::RawValue;
use tower::{Layer, Service};

use crate::{MARKER_PREFIX, Marker, Partial, Rendered};

type Head = Arc<dyn Fn() -> String + Send + Sync>;
type Share = Arc<dyn Fn(&Extensions) -> Result<Vec<u8>, serde_json::Error> + Send + Sync>;

#[derive(Clone)]
struct Config {
    root: Arc<str>,
    version: Arc<str>,
    head: Option<Head>,
    share: Option<Share>,
}

/// The Inertia layer. Put it innermost (after session, csrf and auth), so it
/// sees the handler's response first.
#[derive(Clone)]
pub struct InertiaLayer {
    cfg: Config,
}

pub fn layer() -> InertiaLayer {
    InertiaLayer {
        cfg: Config { root: DEFAULT_ROOT.into(), version: "".into(), head: None, share: None },
    }
}

const DEFAULT_ROOT: &str = r#"<!doctype html>
<html>
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">{{ head }}</head>
<body>{{ inertia }}</body>
</html>"#;

impl InertiaLayer {
    /// The HTML shell for first visits, read now. `{{ head }}` becomes the
    /// `head` output, `{{ inertia }}` the page. Panics if the file cannot be
    /// read: that is a startup bug.
    pub fn root_template(mut self, path: &str) -> Self {
        let html = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("inertia root template {path}: {e}"));
        assert!(html.contains("{{ inertia }}"), "inertia root template {path} has no `{{{{ inertia }}}}`");
        self.cfg.root = html.into();
        self
    }

    /// The asset version. A visit from an older version reloads the page.
    pub fn version(mut self, version: &str) -> Self {
        self.cfg.version = version.into();
        self
    }

    /// Tags for `{{ head }}`, built per first visit (Vite dev tags can change).
    pub fn head(mut self, f: impl Fn() -> String + Send + Sync + 'static) -> Self {
        self.cfg.head = Some(Arc::new(f));
        self
    }

    /// Props for every page, from request extensions. A page prop of the same
    /// name wins.
    pub fn share<T: Serialize>(mut self, f: impl Fn(&Extensions) -> T + Send + Sync + 'static) -> Self {
        self.cfg.share = Some(Arc::new(move |ext| serde_json::to_vec(&f(ext))));
        self
    }
}

impl<S> Layer<S> for InertiaLayer {
    type Service = InertiaService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        InertiaService { inner, cfg: self.cfg.clone() }
    }
}

#[derive(Clone)]
pub struct InertiaService<S> {
    inner: S,
    cfg: Config,
}

impl<S, ReqB> Service<Request<ReqB>> for InertiaService<S>
where
    S: Service<Request<ReqB>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send,
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
        let cfg = self.cfg.clone();

        let method = req.method().clone();
        let inertia = req.headers().contains_key("x-inertia");
        let url = req.uri().path_and_query().map_or("/", |p| p.as_str()).to_owned();

        if inertia && method == Method::GET {
            let theirs = req.headers().get("x-inertia-version").and_then(|v| v.to_str().ok()).unwrap_or("");
            if theirs != &*cfg.version {
                return Box::pin(async move { Ok(version_mismatch(&url, &cfg.version)) });
            }
        }

        let partial = if inertia { Partial::from_headers(req.headers()) } else { None };
        let shared_ext = cfg.share.is_some().then(|| req.extensions().clone());
        #[cfg(feature = "validate")]
        let back = errors::Back::capture(&req, inertia);

        Box::pin(async move {
            let mut res = inner.call(req).await?;

            #[cfg(feature = "validate")]
            if let Some(redirect) = back.redirect_on_errors(&res) {
                return Ok(vary(redirect));
            }
            #[cfg(feature = "validate")]
            let errors = back.flashed_errors();
            #[cfg(not(feature = "validate"))]
            let errors: Option<Box<RawValue>> = None;

            if res.status() == StatusCode::FOUND && matches!(method, Method::PUT | Method::PATCH | Method::DELETE) {
                *res.status_mut() = StatusCode::SEE_OTHER;
            }
            let Some(rendered) = res.extensions_mut().remove::<Rendered>() else {
                return Ok(vary(res));
            };

            let shared = match (&cfg.share, &shared_ext) {
                (Some(f), Some(ext)) => f(ext).ok(),
                _ => None,
            };
            let page = match page_json(&rendered, &url, &cfg.version, partial, shared.as_deref(), errors) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("page `{}` could not be built: {e}", rendered.component);
                    return Ok(StatusCode::INTERNAL_SERVER_ERROR.into_response());
                }
            };

            let (mut parts, _) = res.into_parts();
            if parts.status == StatusCode::INTERNAL_SERVER_ERROR {
                parts.status = StatusCode::OK;
            }
            let body = if inertia {
                parts.headers.insert("x-inertia", HeaderValue::from_static("true"));
                parts.headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
                body(page)
            } else {
                parts.headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
                body(html(&cfg, &page))
            };
            Ok(vary(Response::from_parts(parts, body)))
        })
    }
}

fn vary(mut res: Response<Body>) -> Response<Body> {
    res.headers_mut().append(header::VARY, HeaderValue::from_static("X-Inertia"));
    res
}

/// 409 + `X-Inertia-Location`: the client does a full reload of `url`.
fn version_mismatch(url: &str, version: &str) -> Response<Body> {
    let mut res = StatusCode::CONFLICT.into_response();
    if let Ok(v) = HeaderValue::try_from(url) {
        res.headers_mut().insert("x-inertia-location", v);
    }
    if let Ok(v) = HeaderValue::try_from(version) {
        res.headers_mut().insert("x-inertia-version", v);
    }
    res
}

#[derive(Serialize)]
struct PageJson<'a> {
    component: &'a str,
    props: BTreeMap<&'a str, &'a RawValue>,
    url: &'a str,
    version: &'a str,
    #[serde(rename = "deferredProps", skip_serializing_if = "BTreeMap::is_empty")]
    deferred: BTreeMap<&'a str, Vec<&'a str>>,
}

enum Kind<'a> {
    Regular,
    Optional,
    Defer(&'a str),
}

fn kind(raw: &RawValue) -> Kind<'_> {
    if !raw.get().starts_with(MARKER_PREFIX) {
        return Kind::Regular;
    }
    match serde_json::from_str::<Marker>(raw.get()) {
        Ok(Marker { kind: "defer", group }) => Kind::Defer(group.unwrap_or("default")),
        Ok(Marker { kind: "optional", .. }) => Kind::Optional,
        _ => Kind::Regular,
    }
}

/// Top-level props as raw JSON slices: filtered by key without re-serializing values.
fn props_of(bytes: &[u8]) -> Result<BTreeMap<&str, &RawValue>, serde_json::Error> {
    if bytes == b"null" {
        return Ok(BTreeMap::new()); // unit struct: no props
    }
    serde_json::from_slice(bytes)
}

fn page_json(
    r: &Rendered,
    url: &str,
    version: &str,
    partial: Option<Partial>,
    shared: Option<&[u8]>,
    errors: Option<Box<RawValue>>,
) -> Result<Vec<u8>, serde_json::Error> {
    let empty: &RawValue = serde_json::from_str("{}")?;
    let errors_raw = errors.as_deref().unwrap_or(empty);

    let mut all = shared.map(props_of).transpose()?.unwrap_or_default();
    all.insert("errors", errors_raw);
    all.extend(props_of(&r.props)?);

    // A partial reload for another component is a full visit.
    let partial = partial.filter(|p| p.component == r.component);
    let mut props = BTreeMap::new();
    let mut deferred: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (key, raw) in all {
        let include = match (kind(raw), &partial) {
            (_, _) if key == "errors" => true, // always prop
            (Kind::Regular, None) => true,
            (Kind::Regular, Some(p)) => p.selects(key),
            (Kind::Defer(group), None) => {
                deferred.entry(group).or_default().push(key);
                false
            }
            (Kind::Optional, _) | (Kind::Defer(_), Some(_)) => false,
        };
        if include {
            props.insert(key, raw);
        }
    }
    serde_json::to_vec(&PageJson { component: &r.component, props, url, version, deferred })
}

fn html(cfg: &Config, page: &[u8]) -> String {
    // `<` escaped so the JSON cannot close the script element. No HTML entities:
    // browsers do not decode them inside <script>.
    let json = String::from_utf8_lossy(page).replace('<', "\\u003c");
    let app = format!(r#"<script data-page="app" type="application/json">{json}</script><div id="app"></div>"#);
    let head = cfg.head.as_ref().map(|f| f()).unwrap_or_default();
    cfg.root.replace("{{ head }}", &head).replace("{{ inertia }}", &app)
}

#[cfg(feature = "validate")]
mod errors {
    use std::collections::BTreeMap;

    use route::{
        Body, IntoResponse, Redirect,
        http::{Request, Response, header},
    };
    use serde_json::value::RawValue;
    use session::Session;
    use validate::Errors;

    const KEY: &str = "errors";

    /// What the layer needs from the request to redirect back with errors.
    pub(super) struct Back {
        session: Option<Session>,
        referer: Option<String>,
        inertia: bool,
    }

    impl Back {
        pub(super) fn capture<B>(req: &Request<B>, inertia: bool) -> Self {
            Back {
                session: req.extensions().get::<Session>().cloned(),
                referer: req.headers().get(header::REFERER).and_then(|v| v.to_str().ok()).map(str::to_owned),
                inertia,
            }
        }

        /// On an Inertia visit, a 422 with `Errors` becomes a redirect back,
        /// with the first message per field flashed for the next page.
        pub(super) fn redirect_on_errors(&self, res: &Response<Body>) -> Option<Response<Body>> {
            let (true, Some(session), Some(errors)) = (self.inertia, &self.session, res.extensions().get::<Errors>()) else {
                return None;
            };
            let first: BTreeMap<&str, &str> = errors.iter().filter_map(|(f, m)| Some((f, m.first()?.as_ref()))).collect();
            session.flash(KEY, &first).ok()?;
            Some(Redirect::to(self.referer.clone().unwrap_or_else(|| "/".into())).into_response())
        }

        pub(super) fn flashed_errors(&self) -> Option<Box<RawValue>> {
            let errors: BTreeMap<String, String> = self.session.as_ref()?.get(KEY).ok()??;
            serde_json::value::to_raw_value(&errors).ok()
        }
    }
}
