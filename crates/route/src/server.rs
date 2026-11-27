use std::{convert::Infallible, io, net::SocketAddr, time::Duration};

use http::Request;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto,
};
use tokio::net::{TcpListener, TcpStream};

use crate::{BoxError, ClientAddr, Routes};

/// Serve HTTP/1 and HTTP/2 on `listener`. Runs until the process stops.
/// ponytail: no graceful shutdown; add a shutdown signal when deploys need draining.
pub async fn serve(listener: TcpListener, routes: Routes) -> io::Result<()> {
    loop {
        let (stream, addr) = accept(&listener).await;
        let routes = routes.clone();
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
                let routes = routes.clone();
                async move {
                    let mut req = req.map(|b| b.map_err(|e| Box::new(e) as BoxError).boxed_unsync());
                    req.extensions_mut().insert(ClientAddr(addr));
                    Ok::<_, Infallible>(routes.handle(req).await)
                }
            });
            if let Err(e) = auto::Builder::new(TokioExecutor::new()).serve_connection(TokioIo::new(stream), svc).await {
                tracing::debug!("connection error: {e}");
            }
        });
    }
}

/// Accept the next connection, retrying on errors the way hyper 0.14 and axum do.
async fn accept(listener: &TcpListener) -> (TcpStream, SocketAddr) {
    loop {
        match listener.accept().await {
            Ok(conn) => return conn,
            // The peer gave up before we accepted; the listener is fine.
            Err(e) if matches!(e.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionReset) => {}
            // Usually EMFILE: out of file descriptors. Retrying at once would spin;
            // wait for connections to close.
            Err(e) => {
                tracing::error!("accept error: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}
