//! Commands for the `command` crate: `route:list` and, with `server`, `serve`.

use std::convert::Infallible;

use crate::Routes;

#[derive(clap::Args)]
pub struct ListArgs {}

/// `route:list`: method, path, name and middleware for every route.
pub fn list<S: 'static>(routes: &Routes) -> impl AsyncFnOnce(&S, ListArgs) -> Result<(), Infallible> + use<S> {
    let routes = routes.clone();
    async move |_: &S, _: ListArgs| {
        print!("{}", table(&routes));
        Ok(())
    }
}

fn table(routes: &Routes) -> String {
    let rows: Vec<[String; 4]> = routes
        .table()
        .map(|r| [r.method.to_string(), r.path.clone(), r.name.clone().unwrap_or_default(), r.layers.join(", ")])
        .collect();
    let header = ["METHOD", "PATH", "NAME", "MIDDLEWARE"].map(String::from);
    let mut width = [0; 4];
    for row in std::iter::once(&header).chain(&rows) {
        for (w, cell) in width.iter_mut().zip(row) {
            *w = (*w).max(cell.len());
        }
    }
    let mut out = String::new();
    for row in std::iter::once(&header).chain(&rows) {
        let line: Vec<String> = row.iter().zip(width).map(|(c, w)| format!("{c:w$}")).collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    out
}

#[cfg(feature = "server")]
#[derive(clap::Args)]
pub struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:3000")]
    pub addr: String,
}

/// `serve`: bind `--addr` and serve the routes.
#[cfg(feature = "server")]
pub fn serve<S: 'static>(routes: &Routes) -> impl AsyncFnOnce(&S, ServeArgs) -> std::io::Result<()> + use<S> {
    let routes = routes.clone();
    async move |_: &S, args: ServeArgs| {
        let listener = tokio::net::TcpListener::bind(&args.addr).await?;
        println!("listening on http://{}", listener.local_addr()?);
        crate::serve(listener, routes).await
    }
}
