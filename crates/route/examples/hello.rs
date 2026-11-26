//! `cargo run -p route --example hello --features server`, then
//! `curl localhost:3000/hello/ada` and `curl -i localhost:3000/go`.

use http::HeaderValue;
use route::{Body, Cx, Path, Redirect, Router};

struct App {
    greeting: &'static str,
}

async fn hello(cx: Cx<App>, Path(name): Path<String>) -> String {
    format!("{}, {name}!\n", cx.greeting)
}

async fn go(_: ()) -> Redirect {
    Redirect::to_route("hello").params(&[("name", "world")])
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut r = Router::new();
    r.get("/hello/{name}", hello).name("hello");
    r.group("/", |r| {
        r.layer(tower::util::MapResponseLayer::new(|mut res: http::Response<Body>| {
            res.headers_mut().insert("x-example", HeaderValue::from_static("hello"));
            res
        }));
        r.get("/go", go);
    });
    let routes = r.build(App { greeting: "Hello" }).expect("valid route table");

    for row in routes.table() {
        println!("{} {} {:?}", row.method, row.path, row.name);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    println!("listening on http://{}", listener.local_addr()?);
    route::serve(listener, routes).await
}
