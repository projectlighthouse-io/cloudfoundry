//! `art make:*`: create one file and add one `mod` line. Never rewrite code.

use std::{fs, io::Write, path::Path};

pub enum Kind {
    Handler,
    Action,
    Request,
    Middleware,
    Page,
    Response,
    Job,
    Event { listener: bool },
    Command,
}

pub fn run(kind: Kind, name: &str) -> Result<(), String> {
    // A page is a path (`runs/show`): the Vue file mirrors it, the Rust module flattens it.
    let path = name.trim_matches('/').to_owned();
    let snake = snake_case(&path.replace('/', "_"));
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
        Kind::Page => (
            "responses",
            PAGE_PROPS,
            "Render it from a handler:\n    inertia::render(\"{path}\", responses::{snake}::{pascal} { title: \"...\".into() })",
        ),
        Kind::Response => ("responses", RESPONSE, "Build it from a domain value with `{pascal}::from(&value)`."),
        Kind::Job => (
            "jobs",
            JOB,
            "Register it in src/jobs/mod.rs:\n    w.register::<{snake}::{pascal}>()\nPush it:\n    app.queue.push(jobs::{snake}::{pascal} {}).await?",
        ),
        Kind::Event { listener: false } => ("events", EVENT, "Emit it:\n    app.events.emit(&app, events::{snake}::{pascal} {}).await?"),
        Kind::Event { listener: true } => (
            "events",
            EVENT_WITH_LISTENER,
            "Register the listener in src/events/mod.rs:\n    b.on({snake}::on_{snake})\nEmit it:\n    app.events.emit(&app, events::{snake}::{pascal} {}).await?",
        ),
        Kind::Command => (
            "commands",
            COMMAND,
            "Register it in src/commands/mod.rs (names are `group:verb`):\n    .add(\"app:{snake}\", {snake}::{snake})",
        ),
        Kind::Middleware => (
            "middleware",
            MIDDLEWARE,
            "Add it to a router or group:\n    r.layer(middleware::{snake}::{pascal}Layer);",
        ),
    };
    // From resources/js/pages/<path>.vue back to resources/js/.
    let up = "../".repeat(path.split('/').count());
    let fill = |s: &str| s.replace("{snake}", &snake).replace("{pascal}", &pascal).replace("{path}", &path).replace("{up}", &up);

    let vue = matches!(kind, Kind::Page).then(|| Path::new("resources/js/pages").join(format!("{path}.vue")));
    if let Some(v) = vue.as_ref().filter(|v| v.exists()) {
        return Err(format!("{} already exists", v.display()));
    }

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
        .and_then(|mut f| writeln!(f, "{sep}pub mod {snake};"))
        .map_err(|e| format!("{}: {e}", mod_rs.display()))?;

    if let Some(v) = &vue {
        if let Some(dir) = v.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        fs::write(v, fill(PAGE_VUE)).map_err(|e| format!("{}: {e}", v.display()))?;
        println!("Created {}", v.display());
        let ts = Path::new("resources/js/types").join(format!("{pascal}.ts"));
        if !ts.exists() {
            fs::create_dir_all("resources/js/types").map_err(|e| e.to_string())?;
            fs::write(&ts, fill(PAGE_TS)).map_err(|e| format!("{}: {e}", ts.display()))?;
        }
    }
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
use validate::Validate;

/// Take `Valid<Json<{pascal}>>` (or `Valid<Form<..>>`) in a handler; invalid
/// input gets a 422 before the handler runs. Rules:
/// https://toolkit.projectlighthouse.io/docs/crates/validate.md
#[derive(Deserialize, Validate)]
pub struct {pascal} {
    #[validate(required, max = 255)]
    pub name: String,
}
"#;

const PAGE_PROPS: &str = r#"use serde::Serialize;
use ts_rs::TS;

/// Props for `resources/js/pages/{path}.vue`. `art dev` exports the
/// TypeScript type when this file changes.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct {pascal} {
    pub title: String,
}
"#;

const PAGE_VUE: &str = r#"<script setup lang="ts">
import type { {pascal} } from '{up}types/{pascal}'

defineProps<{pascal}>()
</script>

<template>
  <main>
    <h1>{{ title }}</h1>
  </main>
</template>
"#;

/// What `ts-rs` writes for the stub, so the page builds before the first export.
const PAGE_TS: &str = r#"// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.

/**
 * Props for `resources/js/pages/{path}.vue`. `art dev` exports the
 * TypeScript type when this file changes.
 */
export type {pascal} = { title: string, };
"#;

const RESPONSE: &str = r#"use serde::Serialize;

/// What a response shows of a domain value. Keeps database rows out of
/// responses: name a row by its `uuid`, never by its internal `id`.
#[derive(Serialize)]
pub struct {pascal} {
    // pub uuid: db::Uuid<Run>,
    pub name: String,
}

// impl From<&Run> for {pascal} {
//     fn from(run: &Run) -> Self {
//         Self { uuid: run.uuid, name: run.name.clone() }
//     }
// }
"#;

const JOB: &str = r#"use queue::{Job, JobError};
use serde::{Deserialize, Serialize};

use crate::state::App;

/// Runs at least once: make it safe to run twice.
#[derive(Serialize, Deserialize)]
pub struct {pascal} {}

impl Job for {pascal} {
    /// Stored with queued jobs. Keep it stable.
    const NAME: &'static str = "{snake}";
    type Context = App;

    async fn run(self, app: &App) -> Result<(), JobError> {
        let _ = app;
        Ok(())
    }
}
"#;

const EVENT: &str = r#"use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct {pascal} {}
"#;

const EVENT_WITH_LISTENER: &str = r#"use serde::{Deserialize, Serialize};

use crate::state::App;

#[derive(Clone, Serialize, Deserialize)]
pub struct {pascal} {}

pub async fn on_{snake}(app: App, event: {pascal}) -> Result<(), event::Error> {
    let _ = (app, event);
    Ok(())
}
"#;

const COMMAND: &str = r#"use crate::{error::AppError, state::App};

#[derive(clap::Args)]
pub struct {pascal}Args {}

pub async fn {snake}(app: &App, args: {pascal}Args) -> Result<(), AppError> {
    let _ = (app, args);
    Ok(())
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
