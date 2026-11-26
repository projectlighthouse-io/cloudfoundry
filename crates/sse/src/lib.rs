//! Server-sent event responses.
//!
//! ```ignore
//! async fn logs(cx: Cx<App>, (last, Path(id)): (LastEventId, Path<i64>)) -> Sse {
//!     let stream = cx.logs.subscribe(id, last.0).map(|line| Event::new("log").id(&line.seq.to_string()).json(&line));
//!     Sse::new(stream).keep_alive(Duration::from_secs(15))
//! }
//! ```
//!
//! When the client disconnects, the server drops the body, which drops the
//! stream: no task outlives its connection. A serialization error ends the
//! stream; it is logged, not swallowed into an empty event.

use std::{
    fmt::Write as _,
    future::Future as _,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use futures_core::Stream;
use http::{HeaderValue, Response, header};
use http_body::Frame;
use serde::Serialize;
use tokio::time::{Instant, Sleep};

/// One server-sent event.
#[derive(Clone, Debug, Default)]
pub struct Event {
    event: Option<String>,
    id: Option<String>,
    data: Option<String>,
    retry: Option<u64>,
    error: Option<String>,
}

/// Field values cannot span lines: a newline would start a new field.
fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], "")
}

impl Event {
    /// A named event; the browser dispatches it to `addEventListener(name)`.
    pub fn new(name: &str) -> Self {
        Event { event: Some(one_line(name)), ..Event::default() }
    }

    /// An unnamed event; the browser dispatches it to `onmessage`.
    pub fn message() -> Self {
        Event::default()
    }

    /// Text data. Multi-line text becomes several `data:` lines and arrives intact.
    pub fn data(mut self, data: &str) -> Self {
        self.data = Some(data.to_owned());
        self
    }

    /// JSON data. A serialization error ends the stream when this event is sent.
    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        match serde_json::to_string(value) {
            Ok(s) => self.data = Some(s),
            Err(e) => self.error = Some(e.to_string()),
        }
        self
    }

    /// Sent back by the browser as `Last-Event-ID` when it reconnects.
    pub fn id(mut self, id: &str) -> Self {
        self.id = Some(one_line(id));
        self
    }

    /// How long the browser waits before reconnecting.
    pub fn retry(mut self, after: Duration) -> Self {
        self.retry = Some(after.as_millis() as u64);
        self
    }

    fn encode(&self) -> String {
        let mut out = String::new();
        if let Some(e) = &self.event {
            let _ = writeln!(out, "event: {e}");
        }
        if let Some(id) = &self.id {
            let _ = writeln!(out, "id: {id}");
        }
        if let Some(r) = self.retry {
            let _ = writeln!(out, "retry: {r}");
        }
        for line in self.data.as_deref().unwrap_or("").split('\n') {
            let _ = writeln!(out, "data: {}", line.trim_end_matches('\r'));
        }
        out.push('\n');
        out
    }
}

type EventStream = Pin<Box<dyn Stream<Item = Event> + Send>>;

/// A streaming `text/event-stream` response.
pub struct Sse {
    stream: EventStream,
    keep_alive: Option<Duration>,
}

impl Sse {
    pub fn new(stream: impl Stream<Item = Event> + Send + 'static) -> Self {
        Sse { stream: Box::pin(stream), keep_alive: None }
    }

    /// Send a comment after `every` of silence, so proxies keep the connection open.
    pub fn keep_alive(mut self, every: Duration) -> Self {
        self.keep_alive = Some(every);
        self
    }

    /// The response with its body, for servers that do not use `route`.
    pub fn into_http(self) -> Response<SseBody> {
        let mut res = Response::new(SseBody {
            stream: Some(self.stream),
            keep_alive: self.keep_alive.map(|d| (d, Box::pin(tokio::time::sleep(d)))),
        });
        let h = res.headers_mut();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        // nginx buffers responses by default, which would hold events back.
        h.insert("x-accel-buffering", HeaderValue::from_static("no"));
        res
    }
}

/// The body of an [`Sse`] response.
pub struct SseBody {
    stream: Option<EventStream>,
    keep_alive: Option<(Duration, Pin<Box<Sleep>>)>,
}

impl http_body::Body for SseBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        let Some(stream) = this.stream.as_mut() else { return Poll::Ready(None) };

        match stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(event)) => {
                if let Some(e) = &event.error {
                    tracing::error!("sse event failed to serialize, ending stream: {e}");
                    this.stream = None;
                    return Poll::Ready(None);
                }
                if let Some((every, sleep)) = &mut this.keep_alive {
                    sleep.as_mut().reset(Instant::now() + *every);
                }
                Poll::Ready(Some(Ok(Frame::data(Bytes::from(event.encode())))))
            }
            Poll::Ready(None) => {
                this.stream = None;
                Poll::Ready(None)
            }
            Poll::Pending => {
                let Some((every, sleep)) = &mut this.keep_alive else { return Poll::Pending };
                if sleep.as_mut().poll(cx).is_pending() {
                    return Poll::Pending;
                }
                sleep.as_mut().reset(Instant::now() + *every);
                Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b":\n\n")))))
            }
        }
    }
}

#[cfg(feature = "route")]
mod route_impl {
    use std::{future::Future, sync::Arc};

    use http_body_util::BodyExt;
    use route::{
        Body, FromRequest, FromRequestParts, IntoResponse,
        http::{Request, Response, request::Parts},
    };

    use crate::Sse;

    impl IntoResponse for Sse {
        fn into_response(self) -> Response<Body> {
            self.into_http().map(|b| b.map_err(|e| match e {}).boxed_unsync())
        }
    }

    /// The `Last-Event-ID` a reconnecting browser sends: resume after it.
    #[derive(Clone, Debug, Default)]
    pub struct LastEventId(pub Option<String>);

    impl<S: Send + Sync> FromRequestParts<S> for LastEventId {
        async fn from_request_parts(parts: &mut Parts, _: &Arc<S>) -> Result<Self, Response<Body>> {
            Ok(LastEventId(parts.headers.get("last-event-id").and_then(|v| v.to_str().ok()).map(str::to_owned)))
        }
    }

    impl<S: Send + Sync> FromRequest<S> for LastEventId {
        fn from_request(req: Request<Body>, state: &Arc<S>) -> impl Future<Output = Result<Self, Response<Body>>> + Send {
            route::from_parts(req, state)
        }
    }
}

#[cfg(feature = "route")]
pub use route_impl::LastEventId;
