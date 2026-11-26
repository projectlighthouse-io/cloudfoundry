#![cfg(feature = "route")]

use route::{
    Json, Router, body,
    http::{Request, StatusCode, header},
};
use serde::Deserialize;
use validate::{Errors, Valid, Validate, required};

#[derive(Deserialize)]
struct New {
    name: String,
}

impl Validate for New {
    fn validate(&self, e: &mut Errors) {
        e.check("name", required(&self.name));
    }
}

async fn store(input: Valid<Json<New>>) -> String {
    input.name.clone()
}

#[tokio::test]
async fn valid_input_reaches_the_handler_and_invalid_gets_422() {
    let mut r = Router::new();
    r.post("/", store);
    let routes = r.build(()).unwrap();
    let send = |json: &'static str| {
        let req = Request::post("/").header(header::CONTENT_TYPE, "application/json").body(body(json)).unwrap();
        routes.handle(req)
    };

    let ok = send(r#"{"name":"ci"}"#).await;
    assert_eq!(ok.status(), StatusCode::OK);

    let bad = send(r#"{"name":" "}"#).await;
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad.extensions().get::<Errors>().unwrap().get("name"), ["This field is required."]);
    let bytes = http_body_util::BodyExt::collect(bad.into_body()).await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["errors"]["name"][0], "This field is required.");
}
