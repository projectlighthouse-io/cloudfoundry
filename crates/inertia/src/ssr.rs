//! Server-side rendering: the page object goes to the Inertia SSR server,
//! which answers `{"head": [...], "body": "..."}`. Any failure falls back to
//! client-side rendering: SSR is an optimisation, never a reason to fail.

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Uri, header};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use serde::Deserialize;

#[derive(Clone)]
pub(crate) struct Ssr {
    render: Uri,
    client: Client<HttpConnector, Full<Bytes>>,
    pub(crate) timeout: Duration,
}

#[derive(Deserialize)]
struct Rendered {
    head: Vec<String>,
    body: String,
}

impl Ssr {
    /// `base` is the SSR server, e.g. `http://127.0.0.1:13714`.
    /// `None` (with an error logged) when `base` is not a URL: pages then
    /// render on the client, as they do whenever SSR fails.
    pub(crate) fn new(base: &str) -> Option<Self> {
        match format!("{}/render", base.trim_end_matches('/')).parse() {
            Ok(render) => Some(Ssr { render, client: Client::builder(TokioExecutor::new()).build_http(), timeout: Duration::from_secs(2) }),
            Err(e) => {
                tracing::error!("inertia ssr url {base}: {e}; rendering on the client");
                None
            }
        }
    }

    /// `(head, body)` HTML, or `None` to render on the client.
    pub(crate) async fn render(&self, page: &[u8]) -> Option<(String, String)> {
        let attempt = async {
            let req = Request::post(self.render.clone())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::copy_from_slice(page)))
                .map_err(|e| e.to_string())?;
            let res = self.client.request(req).await.map_err(|e| e.to_string())?;
            let status = res.status();
            let bytes = res.into_body().collect().await.map_err(|e| e.to_string())?.to_bytes();
            if !status.is_success() {
                return Err(format!("status {status}: {}", String::from_utf8_lossy(&bytes)));
            }
            serde_json::from_slice::<Rendered>(&bytes).map_err(|e| e.to_string())
        };
        match tokio::time::timeout(self.timeout, attempt).await {
            Ok(Ok(r)) => Some((r.head.concat(), r.body)),
            Ok(Err(e)) => {
                tracing::warn!("ssr failed, rendering on the client: {e}");
                None
            }
            Err(_) => {
                tracing::warn!("ssr timed out after {:?}, rendering on the client", self.timeout);
                None
            }
        }
    }
}
