//! `art make:*`: create one file and add one `mod` line. Never rewrite code.

use std::{fs, io::Write, path::Path};

pub enum Kind {
    Handler,
    Action,
    Request,
    Middleware,
}

pub fn run(kind: Kind, name: &str) -> Result<(), String> {
    let snake = snake_case(name);
    let valid = snake.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && snake.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(format!("`{name}` is not a valid name: use letters, digits and `_`"));
    }
    let pascal = pascal_case(&snake);

    let (dir, template, hint) = match kind {
        Kind::Handler => (
            "handlers",
            HANDLER,
            "Add to src/routes.rs:\n    r.get(\"/{snake}\", handlers::{snake}::{snake}).name(\"{snake}\");",
        ),
        Kind::Action => (
            "actions",
            ACTION,
            "Build it in src/state.rs and call `.run(input).await`, or route it with `route::action(...)`.",
        ),
        Kind::Request => ("requests", REQUEST, "Take it in a handler as `Valid<Form<requests::{snake}::{pascal}>>`."),
        Kind::Middleware => (
            "middleware",
            MIDDLEWARE,
            "Add it to a router or group:\n    r.layer(middleware::{snake}::{pascal}Layer);",
        ),
    };
    let fill = |s: &str| s.replace("{snake}", &snake).replace("{pascal}", &pascal);

    let dir = Path::new("src").join(dir);
    let mod_rs = dir.join("mod.rs");
    if !mod_rs.exists() {
        return Err(format!("{} not found; run this in an app made by `art new`", mod_rs.display()));
    }
    let file = dir.join(format!("{snake}.rs"));
    if file.exists() {
        return Err(format!("{} already exists", file.display()));
    }

    fs::write(&file, fill(template)).map_err(|e| format!("{}: {e}", file.display()))?;

    let existing = fs::read_to_string(&mod_rs).map_err(|e| e.to_string())?;
    let sep = if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" };
    fs::OpenOptions::new()
        .append(true)
        .open(&mod_rs)
        .and_then(|mut f| write!(f, "{sep}pub mod {snake};\n"))
        .map_err(|e| format!("{}: {e}", mod_rs.display()))?;

    println!("Created {}\n\n{}", file.display(), fill(hint));
    Ok(())
}

/// `StartRun`, `start-run` and `start_run` all become `start_run`.
fn snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c == '-' || c == ' ' {
            out.push('_');
        } else if c.is_ascii_uppercase() {
            if i > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn pascal_case(snake: &str) -> String {
    snake
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect()
}

const HANDLER: &str = r#"use route::Cx;

use crate::{error::AppError, state::App};

pub async fn {snake}(cx: Cx<App>, input: ()) -> Result<&'static str, AppError> {
    let _ = (cx, input);
    Ok("{snake}")
}
"#;

const ACTION: &str = r#"use action::Action;

use crate::error::AppError;

pub struct {pascal}Input {}

/// Dependencies are fields, set once at startup.
pub struct {pascal} {}

impl Action for {pascal} {
    type Input = {pascal}Input;
    type Output = Result<(), AppError>;

    async fn run(&self, input: {pascal}Input) -> Result<(), AppError> {
        let {pascal}Input {} = input;
        Ok(())
    }
}
"#;

const REQUEST: &str = r#"use serde::Deserialize;
use validate::{Errors, Validate, required};

#[derive(Deserialize)]
pub struct {pascal} {
    pub name: String,
}

impl Validate for {pascal} {
    fn validate(&self, e: &mut Errors) {
        e.check("name", required(&self.name));
    }
}
"#;

const MIDDLEWARE: &str = r#"use std::task::{Context, Poll};

use route::{
    Body,
    http::{Request, Response},
};
use tower::{Layer, Service};

#[derive(Clone)]
pub struct {pascal}Layer;

impl<S> Layer<S> for {pascal}Layer {
    type Service = {pascal}<S>;

    fn layer(&self, inner: S) -> Self::Service {
        {pascal} { inner }
    }
}

#[derive(Clone)]
pub struct {pascal}<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for {pascal}<S>
where
    S: Service<Request<Body>, Response = Response<Body>>,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> S::Future {
        // Inspect or change the request here. To act on the response too,
        // box an async block around `self.inner.call(req)`.
        self.inner.call(req)
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for n in ["StartRun", "start-run", "start_run", "startRun"] {
            assert_eq!(snake_case(n), "start_run", "{n}");
        }
        assert_eq!(pascal_case("start_run"), "StartRun");
    }
}
