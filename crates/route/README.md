# route

Match a request to a handler.

```toml
[dependencies]
route = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["server", "command"] }
```

```rust
use route::{Cx, Path, Router};

struct App { greeting: String }

async fn hello(cx: Cx<App>, Path(name): Path<String>) -> String {
    format!("{}, {name}", cx.greeting)
}

let mut r = Router::new();
r.get("/hello/{name}", hello).name("hello");
let routes = r.build(App { greeting: "Hi".into() })?;   // fails on duplicate names or conflicting paths
assert_eq!(routes.url("hello", &[("name", "ada")])?, "/hello/ada");
route::serve(tokio::net::TcpListener::bind("127.0.0.1:3000").await?, routes).await?;
```

Handlers are free async functions with up to six inputs (head-only `FromRequestParts`
ones first, at most one body-reading `FromRequest` last) and one output (`IntoResponse`). Groups share a path prefix, a name prefix and
tower layers, composed once at build time. See `examples/hello.rs`.

Outputs: `Json(value)` (`application/json`), `Html(markup)` (`text/html`), `String` / `&str`
(`text/plain`), `Redirect`, `StatusCode`, `(StatusCode, output)` for another status, and
`Result<output, error>` when both sides are outputs.

Body inputs (`Form`, `Json`, `Bytes`) read at most 2 MiB (`DEFAULT_BODY_LIMIT`);
over it is a 413. Raise it per router or group with `r.body_limit(n)`, or per route
with `r.post("/upload", h).body_limit(n)`.

Route model binding: implement `Bind<App>` for a model (the path parameter's name, a key type
that parses from the segment, and a `resolve` lookup), then take `Bound<Machine>` in a handler
on `/machines/{machine}`. A key that does not parse or resolves to nothing is a 404.

## Features

| Feature | Adds |
| --- | --- |
| `server` | `route::serve` over hyper (HTTP/1 and HTTP/2) |
| `command` | `route::command::{list, serve}` for `route:list` and `serve` |
| `action` | `route::action(a)`: an `Action` as a handler |
