use std::{convert::Infallible, time::Duration};

use http::Request;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto,
};
use tokio::net::TcpListener;

use crate::{BoxError, Routes};

/// Serve HTTP/1 and HTTP/2 on `listener` until it fails.
/// ponytail: no graceful shutdown; add a shutdown signal when deploys need draining.
pub async fn serve(listener: TcpListener, routes: Routes) -> std::io::Result<()> {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                // Usually out of file descriptors. Back off instead of spinning.
                eprintln!("accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let routes = routes.clone();
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
                let routes = routes.clone();
                async move {
                    let req = req.map(|b| b.map_err(|e| Box::new(e) as BoxError).boxed_unsync());
                    Ok::<_, Infallible>(routes.handle(req).await)
                }
            });
            let _ = auto::Builder::new(TokioExecutor::new()).serve_connection(TokioIo::new(stream), svc).await;
        });
    }
}
