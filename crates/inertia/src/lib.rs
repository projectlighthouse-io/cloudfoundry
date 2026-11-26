//! Speak the Inertia.js v3 protocol.
//!
//! ```ignore
//! async fn show(cx: Cx<App>, (visit, Path(id)): (Visit, Path<RunId>)) -> Result<Page<RunShow>, AppError> {
//!     let run = cx.runs.find(id).await?;
//!     Ok(inertia::render("runs/show", RunShow {
//!         run: RunView::from(&run),
//!         logs: visit.optional("logs", cx.runs.logs(id)).await,     // only when asked for
//!         artifacts: visit.defer("artifacts", cx.runs.artifacts(id)).await, // after first render
//!     }))
//! }
//!
//! router.layer(inertia::layer()
//!     .root_template("resources/views/app.html")
//!     .version(vite.manifest_hash())
//!     .head(move || vite.tags()));
//! ```
//!
//! `Page<P>` is plain data. The layer sees the request and does the protocol:
//! HTML on a first visit, JSON on an Inertia visit, partial reloads, deferred
//! props, the asset version check and 303s after `PUT`/`PATCH`/`DELETE`.
//! Responses that are not a `Page` pass through.

mod layer;
#[cfg(feature = "ssr")]
mod ssr;

pub use layer::{InertiaLayer, InertiaService, layer};

use std::{borrow::Cow, future::Future, sync::Arc};

use route::{
    Body, FromRequest, FromRequestParts, IntoResponse,
    http::{Request, Response, StatusCode, request::Parts},
};
use serde::{Serialize, Serializer};

/// A page: component name plus typed props.
pub struct Page<P> {
    component: Cow<'static, str>,
    props: P,
}

/// `render("runs/show", props)`. The component name is the page's path under
/// `resources/js/pages/`, without `.vue`.
pub fn render<P: Serialize>(component: impl Into<Cow<'static, str>>, props: P) -> Page<P> {
    Page { component: component.into(), props }
}

/// A serialized page waiting for the layer.
#[derive(Clone)]
pub(crate) struct Rendered {
    pub(crate) component: Cow<'static, str>,
    pub(crate) props: Arc<[u8]>,
}

impl<P: Serialize> IntoResponse for Page<P> {
    /// Props serialize once, here, straight to bytes. The status is 500 until
    /// the layer renders it, so a route without the layer fails loudly.
    fn into_response(self) -> Response<Body> {
        match serde_json::to_vec(&self.props) {
            Ok(props) => {
                let mut res = StatusCode::INTERNAL_SERVER_ERROR.into_response();
                res.extensions_mut().insert(Rendered { component: self.component, props: props.into() });
                res
            }
            Err(e) => {
                tracing::error!("page `{}` props failed to serialize: {e}", self.component);
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

/// How an unloaded prop announces itself to the layer.
#[derive(Serialize, serde::Deserialize)]
pub(crate) struct Marker<'a> {
    #[serde(rename = "$inertia")]
    pub(crate) kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) group: Option<&'a str>,
    #[serde(default, borrow, skip_serializing)]
    pub(crate) mode: Option<&'a str>,
    #[serde(default, borrow, rename = "match", skip_serializing)]
    pub(crate) match_on: Option<Vec<&'a str>>,
    #[serde(default, borrow, skip_serializing)]
    pub(crate) value: Option<&'a serde_json::value::RawValue>,
    #[serde(default, rename = "expiresAt", skip_serializing)]
    pub(crate) expires_at: Option<u64>,
}

impl<'a> Marker<'a> {
    fn of(kind: &'a str, group: Option<&'a str>) -> Self {
        Marker { kind, group, mode: None, match_on: None, value: None, expires_at: None }
    }
}

pub(crate) const MARKER_PREFIX: &str = r#"{"$inertia":"#;

/// A prop sent only when a partial reload asks for it by name. Never on a
/// first visit, never announced.
pub struct Optional<T>(Option<T>);

impl<T> Optional<T> {
    pub fn new() -> Self {
        Optional(None)
    }
    pub fn loaded(value: T) -> Self {
        Optional(Some(value))
    }
}

impl<T> Default for Optional<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Serialize> Serialize for Optional<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            Some(v) => v.serialize(s),
            None => Marker::of("optional", None).serialize(s),
        }
    }
}

/// A prop left out of the first visit and announced in `deferredProps`; the
/// client fetches it right after rendering, one request per group.
pub struct Defer<T> {
    value: Option<T>,
    group: &'static str,
}

impl<T> Defer<T> {
    pub fn new() -> Self {
        Defer { value: None, group: "default" }
    }
    pub fn loaded(value: T) -> Self {
        Defer { value: Some(value), group: "default" }
    }
    /// Props in different groups load in parallel requests.
    pub fn group(mut self, group: &'static str) -> Self {
        self.group = group;
        self
    }
}

impl<T> Default for Defer<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Serialize> Serialize for Defer<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.value {
            Some(v) => v.serialize(s),
            None => Marker::of("defer", Some(self.group)).serialize(s),
        }
    }
}

/// A prop the client merges into what it has on partial reloads (infinite
/// lists, chat logs). A full visit replaces it as usual.
pub struct Merge<T> {
    value: T,
    mode: &'static str,
    match_on: Vec<&'static str>,
}

impl<T> Merge<T> {
    /// Append (the default for lists).
    pub fn new(value: T) -> Self {
        Merge { value, mode: "append", match_on: vec![] }
    }
    pub fn prepend(mut self) -> Self {
        self.mode = "prepend";
        self
    }
    /// Merge nested objects all the way down.
    pub fn deep(mut self) -> Self {
        self.mode = "deep";
        self
    }
    /// Update items with the same `field` in place instead of appending a copy.
    /// For a nested list, a path: `data.id`.
    pub fn match_on(mut self, field: &'static str) -> Self {
        self.match_on.push(field);
        self
    }
}

impl<T: Serialize> Serialize for Merge<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut m = s.serialize_struct("Merge", 4)?;
        m.serialize_field("$inertia", "merge")?;
        m.serialize_field("mode", self.mode)?;
        m.serialize_field("match", &self.match_on)?;
        m.serialize_field("value", &self.value)?;
        m.end()
    }
}

/// A prop the client remembers across visits: sent once, then skipped while
/// the client still has it (`X-Inertia-Except-Once-Props`). Build it with
/// [`Visit::once`] so the work is skipped too.
pub struct Once<T> {
    value: Option<T>,
    expires_at: Option<u64>,
}

impl<T> Once<T> {
    pub fn loaded(value: T) -> Self {
        Once { value: Some(value), expires_at: None }
    }
    pub fn skipped() -> Self {
        Once { value: None, expires_at: None }
    }
    /// Refresh after this time (milliseconds since the Unix epoch).
    pub fn until(mut self, expires_at_ms: u64) -> Self {
        self.expires_at = Some(expires_at_ms);
        self
    }
}

impl<T: Serialize> Serialize for Once<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut m = s.serialize_struct("Once", 3)?;
        m.serialize_field("$inertia", "once")?;
        if let Some(v) = &self.value {
            m.serialize_field("value", v)?;
        }
        if let Some(e) = self.expires_at {
            m.serialize_field("expiresAt", &e)?;
        }
        m.end()
    }
}

/// A partial reload: the client wants some props of the page it is on.
#[derive(Clone, Debug)]
pub(crate) struct Partial {
    pub(crate) component: String,
    pub(crate) only: Option<Vec<String>>,
    pub(crate) except: Vec<String>,
    /// Props the client resets: sent again without a merge label.
    pub(crate) reset: Vec<String>,
}

impl Partial {
    pub(crate) fn from_headers(h: &route::http::HeaderMap) -> Option<Self> {
        let get = |name: &str| h.get(name).and_then(|v| v.to_str().ok());
        let list = |v: &str| v.split(',').map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()).collect::<Vec<_>>();
        Some(Partial {
            component: get("x-inertia-partial-component")?.to_owned(),
            only: get("x-inertia-partial-data").map(list),
            except: get("x-inertia-partial-except").map(list).unwrap_or_default(),
            reset: get("x-inertia-reset").map(list).unwrap_or_default(),
        })
    }

    /// Data first, then except; a key in both is left out. A dotted entry
    /// (`auth.user`) selects its whole top-level prop.
    /// ponytail: no nested filtering; add it if a page sends large nested props.
    pub(crate) fn selects(&self, key: &str) -> bool {
        let hit = |list: &[String]| list.iter().any(|k| k == key || k.strip_prefix(key).is_some_and(|r| r.starts_with('.')));
        self.only.as_deref().is_none_or(hit) && !self.except.iter().any(|k| k == key)
    }
}

/// What this visit asks for. Use it to skip work for props that will not be sent.
#[derive(Clone, Debug, Default)]
pub struct Visit {
    partial: Option<Partial>,
    /// Once props the client still has.
    except_once: Vec<String>,
}

impl Visit {
    /// Whether a partial reload asked for `key`. False on a first visit.
    pub fn wants(&self, key: &str) -> bool {
        self.partial.as_ref().is_some_and(|p| p.selects(key))
    }

    /// Run `load` only if this visit asks for `key`.
    pub async fn optional<T>(&self, key: &str, load: impl Future<Output = T>) -> Optional<T> {
        if self.wants(key) { Optional::loaded(load.await) } else { Optional::new() }
    }

    /// Run `load` unless the client still has once prop `key`. A partial
    /// reload that asks for it always loads it.
    pub async fn once<T>(&self, key: &str, load: impl Future<Output = T>) -> Once<T> {
        let needed = match &self.partial {
            Some(p) => p.selects(key),
            None => !self.except_once.iter().any(|k| k == key),
        };
        if needed { Once::loaded(load.await) } else { Once::skipped() }
    }

    /// Run `load` only on the follow-up request that fetches deferred `key`.
    pub async fn defer<T>(&self, key: &str, load: impl Future<Output = T>) -> Defer<T> {
        if self.wants(key) { Defer::loaded(load.await) } else { Defer::new() }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Visit {
    async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
        let inertia = parts.headers.contains_key("x-inertia");
        let except_once = match parts.headers.get("x-inertia-except-once-props").and_then(|v| v.to_str().ok()) {
            Some(v) if inertia => v.split(',').map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()).collect(),
            _ => vec![],
        };
        Ok(Visit { partial: inertia.then(|| Partial::from_headers(&parts.headers)).flatten(), except_once })
    }
}

impl<S: Send + Sync> FromRequest<S> for Visit {
    fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
        route::from_parts(req, state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_selection() {
        let p = |only: Option<&[&str]>, except: &[&str]| Partial {
            component: "c".into(),
            only: only.map(|o| o.iter().map(|s| s.to_string()).collect()),
            except: except.iter().map(|s| s.to_string()).collect(),
            reset: vec![],
        };
        assert!(p(None, &[]).selects("a"));
        assert!(p(Some(&["a"]), &[]).selects("a") && !p(Some(&["a"]), &[]).selects("b"));
        assert!(!p(None, &["a"]).selects("a"), "except alone");
        assert!(!p(Some(&["a"]), &["a"]).selects("a"), "in both is left out");
        assert!(p(Some(&["auth.user"]), &[]).selects("auth") && !p(Some(&["authx"]), &[]).selects("auth"));
    }

    #[test]
    fn markers() {
        assert_eq!(serde_json::to_string(&Optional::<u8>::new()).unwrap(), r#"{"$inertia":"optional"}"#);
        assert_eq!(serde_json::to_string(&Defer::<u8>::new().group("side")).unwrap(), r#"{"$inertia":"defer","group":"side"}"#);
        assert_eq!(serde_json::to_string(&Defer::loaded(3)).unwrap(), "3");
    }
}
