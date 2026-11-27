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
    encrypt_history: bool,
    #[cfg(feature = "ssr")]
    ssr: Option<Arc<crate::ssr::Ssr>>,
    pages: Option<Arc<str>>,
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
        cfg: Config {
            encrypt_history: false,
            #[cfg(feature = "ssr")]
            ssr: None,
            pages: None,
            root: DEFAULT_ROOT.into(), version: "".into(), head: None, share: None },
    }
}

const DEFAULT_ROOT: &str = r#"<!doctype html>
<html>
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">{{ head }}</head>
<body>{{ inertia }}</body>
</html>"#;

impl InertiaLayer {
    /// The HTML shell for first visits, read now. `{{ head }}` becomes the
    /// `head` output, `{{ inertia }}` the page. If the file cannot be read or
    /// has no `{{ inertia }}`, an error is logged and the built-in shell is used.
    pub fn root_template(mut self, path: &str) -> Self {
        match std::fs::read_to_string(path) {
            Ok(html) if html.contains("{{ inertia }}") => self.cfg.root = html.into(),
            Ok(_) => tracing::error!("inertia root template {path} has no `{{{{ inertia }}}}`; using the built-in one"),
            Err(e) => tracing::error!("inertia root template {path}: {e}; using the built-in one"),
        }
        self
    }

    /// In debug builds, check that `<dir>/<component>.vue` exists before
    /// rendering, so a typo in a component name is a clear error, not a blank
    /// page. Release builds skip the check.
    pub fn pages_dir(mut self, dir: &str) -> Self {
        self.cfg.pages = Some(dir.trim_end_matches('/').into());
        self
    }

    /// Render first visits on the Inertia SSR server at `url`
    /// (`http://127.0.0.1:13714`). Falls back to client rendering if it is
    /// down, slow or failing.
    #[cfg(feature = "ssr")]
    pub fn ssr(mut self, url: &str) -> Self {
        self.cfg.ssr = crate::ssr::Ssr::new(url).map(Arc::new);
        self
    }

    /// Give up on SSR after this long (default 2s) and render on the client.
    #[cfg(feature = "ssr")]
    pub fn ssr_timeout(mut self, timeout: std::time::Duration) -> Self {
        if let Some(s) = self.cfg.ssr.take() {
            let mut s = Arc::unwrap_or_clone(s);
            s.timeout = timeout;
            self.cfg.ssr = Some(Arc::new(s));
        }
        self
    }

    /// Encrypt every page in the browser's history (`Page::encrypt_history`
    /// overrides per page). Pair with `inertia::clear_history` on logout.
    pub fn encrypt_history(mut self, encrypt: bool) -> Self {
        self.cfg.encrypt_history = encrypt;
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
        #[cfg(feature = "session")]
        let session = req.extensions().get::<session::Session>().cloned();

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

            if let Some(dir) = cfg.pages.as_deref().filter(|_| cfg!(debug_assertions)) {
                let file = format!("{dir}/{}.vue", rendered.component);
                if !std::path::Path::new(&file).exists() {
                    tracing::error!("page component `{}` not found at {file}", rendered.component);
                    return Ok(StatusCode::INTERNAL_SERVER_ERROR.into_response());
                }
            }

            let shared = match (&cfg.share, &shared_ext) {
                (Some(f), Some(ext)) => f(ext).ok(),
                _ => None,
            };
            #[cfg(feature = "session")]
            let cleared = session.is_some_and(|s| s.get::<bool>(crate::CLEAR_HISTORY_KEY).ok().flatten().unwrap_or(false));
            #[cfg(not(feature = "session"))]
            let cleared = false;
            let history = History {
                encrypt: rendered.encrypt_history.unwrap_or(cfg.encrypt_history),
                clear: rendered.clear_history || cleared,
            };
            let page = match page_json(&rendered, &url, &cfg.version, partial, shared.as_deref(), errors, history) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("page `{}` could not be built: {e}", rendered.component);
                    return Ok(StatusCode::INTERNAL_SERVER_ERROR.into_response());
                }
            };

            let (mut parts, _) = res.into_parts();
            if parts.status.as_u16() == crate::PENDING {
                parts.status = StatusCode::OK;
            }
            let body = if inertia {
                parts.headers.insert("x-inertia", HeaderValue::from_static("true"));
                parts.headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
                body(page)
            } else {
                parts.headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
                #[cfg(feature = "ssr")]
                if let Some(ssr) = &cfg.ssr {
                    if let Some((head, app)) = ssr.render(&page).await {
                        return Ok(vary(Response::from_parts(parts, body(shell(&cfg, &head, &app)))));
                    }
                }
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
    #[serde(rename = "mergeProps", skip_serializing_if = "Vec::is_empty")]
    merge: Vec<&'a str>,
    #[serde(rename = "prependProps", skip_serializing_if = "Vec::is_empty")]
    prepend: Vec<&'a str>,
    #[serde(rename = "deepMergeProps", skip_serializing_if = "Vec::is_empty")]
    deep_merge: Vec<&'a str>,
    #[serde(rename = "matchPropsOn", skip_serializing_if = "Vec::is_empty")]
    match_on: Vec<String>,
    #[serde(rename = "onceProps", skip_serializing_if = "BTreeMap::is_empty")]
    once: BTreeMap<&'a str, OnceEntry<'a>>,
    #[serde(rename = "scrollProps", skip_serializing_if = "BTreeMap::is_empty")]
    scroll: BTreeMap<&'a str, ScrollEntry<'a>>,
    #[serde(rename = "encryptHistory", skip_serializing_if = "std::ops::Not::not")]
    encrypt_history: bool,
    #[serde(rename = "clearHistory", skip_serializing_if = "std::ops::Not::not")]
    clear_history: bool,
}

#[derive(Serialize)]
struct ScrollEntry<'a> {
    #[serde(rename = "pageName")]
    page_name: &'a str,
    #[serde(rename = "previousPage")]
    previous_page: Option<u64>,
    #[serde(rename = "nextPage")]
    next_page: Option<u64>,
    #[serde(rename = "currentPage")]
    current_page: Option<u64>,
    reset: bool,
}

#[derive(Clone, Copy)]
struct History {
    encrypt: bool,
    clear: bool,
}

#[derive(Serialize)]
struct OnceEntry<'a> {
    prop: &'a str,
    #[serde(rename = "expiresAt")]
    expires_at: Option<u64>,
}

enum Kind<'a> {
    Regular,
    Optional,
    Defer(&'a str),
    Merge { mode: &'a str, match_on: Vec<&'a str>, value: &'a RawValue },
    Scroll { value: &'a RawValue, page_name: &'a str, previous: Option<u64>, next: Option<u64>, current: Option<u64>, match_on: Vec<&'a str> },
    Once { value: Option<&'a RawValue>, expires_at: Option<u64> },
}

fn kind(raw: &RawValue) -> Kind<'_> {
    if !raw.get().starts_with(MARKER_PREFIX) {
        return Kind::Regular;
    }
    match serde_json::from_str::<Marker>(raw.get()) {
        Ok(Marker { kind: "defer", group, .. }) => Kind::Defer(group.unwrap_or("default")),
        Ok(Marker { kind: "optional", .. }) => Kind::Optional,
        Ok(Marker { kind: "merge", mode, match_on, value: Some(value), .. }) => {
            Kind::Merge { mode: mode.unwrap_or("append"), match_on: match_on.unwrap_or_default(), value }
        }
        Ok(Marker { kind: "once", value, expires_at, .. }) => Kind::Once { value, expires_at },
        Ok(Marker { kind: "scroll", value: Some(value), page_name, previous_page, next_page, current_page, match_on, .. }) => Kind::Scroll {
            value,
            page_name: page_name.unwrap_or("page"),
            previous: previous_page,
            next: next_page,
            current: current_page,
            match_on: match_on.unwrap_or_default(),
        },
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
    history: History,
) -> Result<Vec<u8>, serde_json::Error> {
    let empty: &RawValue = serde_json::from_str("{}")?;
    let errors_raw = errors.as_deref().unwrap_or(empty);

    let mut all = shared.map(props_of).transpose()?.unwrap_or_default();
    all.insert("errors", errors_raw);
    all.extend(props_of(&r.props)?);

    // A partial reload for another component is a full visit.
    let partial = partial.filter(|p| p.component == r.component);
    let selected = |key: &str| partial.as_ref().is_none_or(|p| p.selects(key));
    let reset = |key: &str| partial.as_ref().is_some_and(|p| p.reset.iter().any(|k| k == key));
    let mut page = PageJson {
        component: &r.component,
        props: BTreeMap::new(),
        url,
        version,
        deferred: BTreeMap::new(),
        merge: vec![],
        prepend: vec![],
        deep_merge: vec![],
        match_on: vec![],
        once: BTreeMap::new(),
        scroll: BTreeMap::new(),
        encrypt_history: history.encrypt,
        clear_history: history.clear,
    };
    let prepend = partial.as_ref().is_some_and(|p| p.prepend);
    for (key, raw) in all {
        if key == "errors" {
            page.props.insert(key, raw); // always prop
            continue;
        }
        match kind(raw) {
            Kind::Regular if selected(key) => {
                page.props.insert(key, raw);
            }
            Kind::Defer(group) if partial.is_none() => page.deferred.entry(group).or_default().push(key),
            Kind::Merge { mode, match_on, value } if selected(key) => {
                page.props.insert(key, value);
                // A reset prop is sent plain, so the client replaces it.
                if !reset(key) {
                    match mode {
                        "prepend" => page.prepend.push(key),
                        "deep" => page.deep_merge.push(key),
                        _ => page.merge.push(key),
                    }
                    page.match_on.extend(match_on.iter().map(|f| format!("{key}.{f}")));
                }
            }
            Kind::Scroll { value, page_name, previous, next, current, match_on } if selected(key) => {
                page.props.insert(key, value);
                let reset = reset(key);
                page.scroll.insert(key, ScrollEntry { page_name, previous_page: previous, next_page: next, current_page: current, reset });
                if !reset {
                    if prepend { page.prepend.push(key) } else { page.merge.push(key) }
                    page.match_on.extend(match_on.iter().map(|f| format!("{key}.{f}")));
                }
            }
            Kind::Once { value, expires_at } => {
                // Announced either way, so the client keeps remembering it.
                page.once.insert(key, OnceEntry { prop: key, expires_at });
                if let Some(v) = value.filter(|_| selected(key)) {
                    page.props.insert(key, v);
                }
            }
            _ => {}
        }
    }
    serde_json::to_vec(&page)
}

fn html(cfg: &Config, page: &[u8]) -> String {
    // `<` escaped so the JSON cannot close the script element. No HTML entities:
    // browsers do not decode them inside <script>.
    let json = String::from_utf8_lossy(page).replace('<', "\\u003c");
    let app = format!(r#"<script data-page="app" type="application/json">{json}</script><div id="app"></div>"#);
    shell(cfg, "", &app)
}

/// The root template with `{{ head }}` (asset tags, then `extra`) and `{{ inertia }}` filled.
fn shell(cfg: &Config, extra: &str, app: &str) -> String {
    let head = cfg.head.as_ref().map(|f| f()).unwrap_or_default() + extra;
    cfg.root.replace("{{ head }}", &head).replace("{{ inertia }}", app)
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
        /// `X-Inertia-Error-Bag`: the form's errors go under this name.
        bag: Option<String>,
    }

    impl Back {
        pub(super) fn capture<B>(req: &Request<B>, inertia: bool) -> Self {
            Back {
                session: req.extensions().get::<Session>().cloned(),
                referer: req.headers().get(header::REFERER).and_then(|v| v.to_str().ok()).map(str::to_owned),
                inertia,
                bag: req.headers().get("x-inertia-error-bag").and_then(|v| v.to_str().ok()).map(str::to_owned),
            }
        }

        /// On an Inertia visit, a 422 with `Errors` becomes a redirect back,
        /// with the first message per field flashed for the next page.
        pub(super) fn redirect_on_errors(&self, res: &Response<Body>) -> Option<Response<Body>> {
            let (true, Some(session), Some(errors)) = (self.inertia, &self.session, res.extensions().get::<Errors>()) else {
                return None;
            };
            let first: BTreeMap<&str, &str> = errors.iter().filter_map(|(f, m)| Some((f, m.first()?.as_ref()))).collect();
            match &self.bag {
                Some(bag) => session.flash(KEY, &BTreeMap::from([(bag.as_str(), first)])).ok()?,
                None => session.flash(KEY, &first).ok()?,
            }
            Some(Redirect::to(self.referer.clone().unwrap_or_else(|| "/".into())).into_response())
        }

        pub(super) fn flashed_errors(&self) -> Option<Box<RawValue>> {
            // A flat map, or one wrapped in its error bag's name.
            let errors: serde_json::Value = self.session.as_ref()?.get(KEY).ok()??;
            serde_json::value::to_raw_value(&errors).ok()
        }
    }
}
