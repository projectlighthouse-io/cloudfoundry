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

Handlers are free async functions with one input (`FromRequest`, or a tuple of up to
four) and one output (`IntoResponse`). Groups share a path prefix, a name prefix and
tower layers, composed once at build time. See `examples/hello.rs`.

Body inputs (`Form`, `Json`, `Bytes`) read at most 2 MiB (`DEFAULT_BODY_LIMIT`);
over it is a 413. Raise it per router or group with `r.body_limit(n)`, or per route
with `r.post("/upload", h).body_limit(n)`.

## Features

| Feature | Adds |
| --- | --- |
| `server` | `route::serve` over hyper (HTTP/1 and HTTP/2) |
| `command` | `route::command::{list, serve}` for `route:list` and `serve` |
| `action` | `route::action(a)`: an `Action` as a handler |
