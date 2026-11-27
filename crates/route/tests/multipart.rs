#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
#![cfg(feature = "multipart")]

use http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use route::{Multipart, MultipartError, Router, body};

async fn upload(mut form: Multipart) -> Result<String, MultipartError> {
    let mut out = vec![];
    while let Some(field) = form.next_field().await? {
        let name = field.name().unwrap_or("").to_owned();
        let file = field.file_name().map(str::to_owned);
        let bytes = field.bytes().await?;
        out.push(format!("{name}:{}:{}", file.unwrap_or_default(), bytes.len()));
    }
    Ok(out.join(","))
}

fn form(file_len: usize) -> String {
    format!(
        "--XX\r\nContent-Disposition: form-data; name=\"domain\"\r\n\r\nexample.com\r\n\
         --XX\r\nContent-Disposition: form-data; name=\"cert\"; filename=\"cert.pem\"\r\nContent-Type: application/x-pem-file\r\n\r\n{}\r\n\
         --XX--\r\n",
        "x".repeat(file_len)
    )
}

#[tokio::test]
async fn reads_fields_and_files_within_the_limit() {
    let mut r = Router::<()>::new();
    r.post("/up", upload).body_limit(1024);
    let routes = r.build(()).unwrap();
    let send = |b: String, ct: &'static str| {
        routes.handle(Request::builder().method(Method::POST).uri("/up").header(header::CONTENT_TYPE, ct).body(body(b)).unwrap())
    };

    let res = send(form(100), "multipart/form-data; boundary=XX").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.into_body().collect().await.unwrap().to_bytes(), "domain::11,cert:cert.pem:100");

    assert_eq!(send(form(5000), "multipart/form-data; boundary=XX").await.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(send(form(1), "text/plain").await.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}
