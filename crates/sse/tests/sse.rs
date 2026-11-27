#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
use std::time::Duration;

use http_body_util::BodyExt;
use serde::{Serialize, Serializer};
use sse::{Event, Sse};

async fn frames(sse: Sse) -> Vec<String> {
    let mut body = sse.into_http().into_body();
    let mut out = vec![];
    while let Some(f) = body.frame().await {
        out.push(String::from_utf8(f.unwrap().into_data().unwrap().to_vec()).unwrap());
    }
    out
}

#[tokio::test]
async fn encodes_events() {
    let events = vec![
        Event::new("log").id("1").data("line one\nline two"),
        Event::message().json(&[1, 2]),
        Event::new("evil\nname").id("a\r\nb").retry(Duration::from_secs(3)),
    ];
    assert_eq!(
        frames(Sse::new(tokio_stream::iter(events))).await,
        [
            "event: log\nid: 1\ndata: line one\ndata: line two\n\n",
            "data: [1,2]\n\n",
            "event: evilname\nid: ab\nretry: 3000\ndata: \n\n",
        ]
    );
}

struct Bad;
impl Serialize for Bad {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("nope"))
    }
}

#[tokio::test]
async fn serialization_error_ends_the_stream() {
    let events = vec![Event::message().data("a"), Event::message().json(&Bad), Event::message().data("never")];
    assert_eq!(frames(Sse::new(tokio_stream::iter(events))).await, ["data: a\n\n"]);
}

#[tokio::test(start_paused = true)]
async fn keep_alive_while_idle() {
    let mut body = Sse::new(tokio_stream::pending::<Event>()).keep_alive(Duration::from_secs(15)).into_http().into_body();
    let f = body.frame().await.unwrap().unwrap();
    assert_eq!(f.into_data().unwrap(), ":\n\n");
    let f = body.frame().await.unwrap().unwrap();
    assert_eq!(f.into_data().unwrap(), ":\n\n");
}

#[cfg(feature = "route")]
#[tokio::test]
async fn as_a_handler_output() {
    use route::{Router, empty, http::Request};
    use sse::LastEventId;

    let mut r = Router::new();
    r.get("/events", |last: LastEventId| async move {
        Sse::new(tokio_stream::iter([Event::message().data(&last.0.unwrap_or_default())]))
    });
    let routes = r.build(()).unwrap();
    let res = routes.handle(Request::get("/events").header("last-event-id", "41").body(empty()).unwrap()).await;
    assert_eq!(res.headers()["content-type"], "text/event-stream");
    assert_eq!(res.into_body().collect().await.unwrap().to_bytes(), "data: 41\n\n");
}
