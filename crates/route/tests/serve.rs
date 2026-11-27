#![cfg(feature = "server")]

use route::{ClientAddr, Router};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn ip(ClientAddr(addr): ClientAddr) -> String {
    addr.ip().to_string()
}

#[tokio::test]
async fn serve_records_the_client_address() {
    let mut r = Router::new();
    r.get("/", ip);
    let routes = r.build(()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(route::serve(listener, routes));

    let mut conn = tokio::net::TcpStream::connect(addr).await.unwrap();
    conn.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut res = String::new();
    conn.read_to_string(&mut res).await.unwrap();
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
    assert!(res.ends_with("127.0.0.1"), "{res}");
}
