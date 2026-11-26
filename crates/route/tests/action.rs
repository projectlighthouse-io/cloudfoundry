#![cfg(feature = "action")]

use action::Action;
use http::Request;
use http_body_util::BodyExt;
use route::{Path, Router, empty};

struct Double {
    factor: u32,
}

impl Action for Double {
    type Input = Path<u32>;
    type Output = String;

    async fn run(&self, Path(n): Path<u32>) -> String {
        (n * self.factor).to_string()
    }
}

#[tokio::test]
async fn action_as_handler() {
    let mut r = Router::new();
    r.get("/double/{n}", route::action(Double { factor: 2 }));
    let routes = r.build(()).unwrap();
    let res = routes.handle(Request::get("/double/21").body(empty()).unwrap()).await;
    assert_eq!(res.into_body().collect().await.unwrap().to_bytes(), "42");
}
