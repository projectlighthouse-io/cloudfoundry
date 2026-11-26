# sse

Server-sent event responses.

```toml
[dependencies]
sse = { git = "https://github.com/projectlighthouse-io/cloudfoundry", features = ["route"] }
```

```rust
async fn logs(last: sse::LastEventId) -> sse::Sse {
    let stream = subscribe(last.0).map(|line| sse::Event::new("log").id(&line.seq.to_string()).json(&line));
    sse::Sse::new(stream).keep_alive(Duration::from_secs(15))
}
```

The stream is dropped when the client disconnects. A serialization error ends the
stream and is logged.

## Features

| Feature | Adds |
| --- | --- |
| `route` | `Sse` as a handler output, `LastEventId` as an input |
