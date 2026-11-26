//! `art new <name>`: the app tree and its `main.rs` wiring.

use std::{fs, path::PathBuf};

/// Where the generated app gets the toolkit crates from.
enum Source {
    Path(PathBuf),
    Git(String),
}

impl Source {
    fn dep(&self, krate: &str) -> String {
        match self {
            Source::Path(root) => format!("path = {:?}", root.join("crates").join(krate)),
            Source::Git(url) => format!("git = {url:?}"),
        }
    }
}

pub fn run(name: &str, path: Option<PathBuf>, git: Option<String>) -> Result<(), String> {
    let valid = name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !valid {
        return Err(format!("`{name}` is not a valid app name: use lowercase letters, digits, `-` and `_`"));
    }

    let source = match (path, git) {
        (Some(p), _) => Source::Path(fs::canonicalize(&p).map_err(|e| format!("{}: {e}", p.display()))?),
        (_, Some(url)) => Source::Git(url),
        // ponytail: falls back to the checkout art was built from; set --git once the repo has a remote.
        (None, None) => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
            match fs::canonicalize(&root) {
                Ok(root) if root.join("crates/route").exists() => Source::Path(root),
                _ => return Err("pass --git <url> or --path <checkout> to locate the toolkit crates".into()),
            }
        }
    };

    let dir = PathBuf::from(name);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }

    let cargo_toml = CARGO_TOML
        .replace("{route}", &source.dep("route"))
        .replace("{db}", &source.dep("db"))
        .replace("{command}", &source.dep("command"))
        .replace("{action}", &source.dep("action"))
        .replace("{validate}", &source.dep("validate"))
        .replace("{securecookie}", &source.dep("securecookie"))
        .replace("{session}", &source.dep("session"))
        .replace("{csrf}", &source.dep("csrf"))
        .replace("{inertia}", &source.dep("inertia"))
        .replace("{vite}", &source.dep("vite"))
        .replace("{queue}", &source.dep("queue"))
        .replace("{event}", &source.dep("event"))
        .replace("{sse}", &source.dep("sse"));
    let env = format!("APP_NAME={name}\nAPP_KEY={}\nDATABASE_URL=postgres://localhost/{name}\n", crate::generate_key());

    let files: &[(&str, &str)] = &[
        ("Cargo.toml", &cargo_toml),
        (".gitignore", "/target\n.env\n/node_modules\n/public/build\n/public/hot\n"),
        (".env.example", ENV),
        (".env", &env),
        ("migrations/.gitkeep", ""),
        ("src/main.rs", MAIN),
        ("src/state.rs", STATE),
        ("src/config.rs", CONFIG),
        ("src/error.rs", ERROR),
        ("src/routes.rs", ROUTES),
        ("src/handlers/mod.rs", HANDLERS),
        ("src/actions/mod.rs", ""),
        ("src/requests/mod.rs", ""),
        ("src/middleware/mod.rs", ""),
        ("src/responses/mod.rs", "pub mod home;\n"),
        ("src/jobs/mod.rs", JOBS),
        ("src/events/mod.rs", EVENTS),
        ("src/responses/home.rs", HOME_PROPS),
        ("package.json", PACKAGE_JSON),
        ("vite.config.ts", VITE_CONFIG),
        ("resources/js/app.ts", APP_TS),
        ("resources/js/pages/home.vue", HOME_VUE),
        ("resources/views/app.html", APP_HTML),
        ("public/.gitkeep", ""),
        ("src/commands/mod.rs", COMMANDS),
    ];
    for (file, contents) in files {
        let file = dir.join(file);
        fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&file, contents.replace("{name}", name)).map_err(|e| format!("{}: {e}", file.display()))?;
    }

    println!(
        "Created {name}.\n\n  cd {name}\n  # check DATABASE_URL in .env\n  npm install\n  art publish session   # sessions table migration\n  art publish queue     # jobs tables migration\n  art migrate\n  art dev"
    );
    Ok(())
}

const CARGO_TOML: &str = r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
action = { {action} }
command = { {command}, features = ["action"] }
db = { {db}, features = ["postgres"] }
route = { {route}, features = ["server", "command", "action"] }
validate = { {validate}, features = ["route"] }
csrf = { {csrf}, features = ["route"] }
securecookie = { {securecookie} }
session = { {session}, features = ["postgres", "route", "csrf", "command"] }
inertia = { {inertia}, features = ["validate"] }
vite = { {vite} }
queue = { {queue}, features = ["postgres", "command"] }
event = { {event}, features = ["queue", "command"] }
sse = { {sse}, features = ["route"] }

clap = { version = "4", features = ["derive"] }
dotenvy = "0.15"
serde = { version = "1", features = ["derive"] }
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "postgres", "macros", "migrate"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["fs"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
"#;

const ENV: &str = "APP_NAME={name}\nAPP_KEY=\nDATABASE_URL=postgres://localhost/{name}\n";

const MAIN: &str = r#"mod actions;
mod commands;
mod config;
mod error;
mod events;
mod handlers;
mod jobs;
mod middleware;
mod requests;
mod responses;
mod routes;
mod state;

use std::sync::Arc;

use db::Db;
use event::Events;
use queue::Queue;
use route::Router;
use securecookie::{Codec, Keys};
use session::{PostgresStore, Session};
use tower_http::services::ServeDir;
use vite::Vite;
use tracing_subscriber::EnvFilter;

use crate::{config::Config, error::AppError, state::{App, Primary}};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cfg = Config::from_env()?;
    let codec = Codec::new(Keys::parse(&cfg.app_key).map_err(|e| AppError::Config(format!("APP_KEY: {e}")))?);

    let vite = Vite::load("public", &["resources/js/app.ts"]).map_err(|e| AppError::Config(e.to_string()))?;
    let asset_version = vite.manifest_hash().to_owned();

    let db: Db<Primary> = Db::connect(&cfg.database_url).await?;
    let sessions = PostgresStore::new(db.pool().clone());

    let queue = Queue::new(queue::PostgresStore::new(db.pool().clone()));
    let events = Arc::new(events::register(Events::builder().queue(&queue)).build());

    let app = App::new(cfg, db, queue.clone(), events.clone());

    let router = routes::register(Router::new())
        .layer(session::layer(sessions.clone(), codec))
        .layer(csrf::layer::<Session>())
        .layer(
            inertia::layer()
                .root_template("resources/views/app.html")
                .version(&asset_version)
                .head(move || vite.tags()),
        )
        .fallback(ServeDir::new("public"))
        .build(app.clone())?;

    commands::register(&router, sessions, queue, events).run(app, std::env::args()).await;
    Ok(())
}
"#;

const STATE: &str = r#"use std::sync::Arc;

use db::Db;
use event::Events;
use queue::{PostgresStore, Queue};

use crate::config::Config;

db::handle!(pub Primary: Postgres);

/// Everything a handler or command needs. Cheap to clone.
#[derive(Clone)]
pub struct App {
    pub cfg: Arc<Config>,
    pub db: Db<Primary>,
    pub queue: Queue<PostgresStore>,
    pub events: Arc<Events<App>>,
}

impl App {
    pub fn new(cfg: Config, db: Db<Primary>, queue: Queue<PostgresStore>, events: Arc<Events<App>>) -> Self {
        Self { cfg: Arc::new(cfg), db, queue, events }
    }
}
"#;

const CONFIG: &str = r#"use crate::error::AppError;

/// Typed config, read once from the environment (and `.env` if present).
pub struct Config {
    pub app_name: String,
    pub app_key: String,
    pub database_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self, AppError> {
        let _ = dotenvy::dotenv();
        Ok(Self {
            app_name: std::env::var("APP_NAME").unwrap_or_else(|_| "{name}".into()),
            app_key: required("APP_KEY")?,
            database_url: required("DATABASE_URL")?,
        })
    }
}

fn required(key: &str) -> Result<String, AppError> {
    std::env::var(key).map_err(|_| AppError::Config(format!("{key} is not set")))
}
"#;

const ERROR: &str = r#"use std::fmt;

use route::{Body, IntoResponse, http::{Response, StatusCode}};

#[derive(Debug)]
pub enum AppError {
    Config(String),
    Db(sqlx::Error),
    Routes(route::BuildError),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(msg) => write!(f, "config: {msg}"),
            Self::Db(e) => write!(f, "database: {e}"),
            Self::Routes(e) => write!(f, "routes: {e}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

impl From<route::BuildError> for AppError {
    fn from(e: route::BuildError) -> Self {
        Self::Routes(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response<Body> {
        tracing::error!("{self}");
        (StatusCode::INTERNAL_SERVER_ERROR, "server error").into_response()
    }
}
"#;

const ROUTES: &str = r#"use route::Router;

use crate::{handlers, state::App};

/// Every route in one place.
pub fn register(mut r: Router<App>) -> Router<App> {
    r.get("/", handlers::home).name("home");
    r.get("/health", handlers::health).name("health");
    r
}
"#;

const HANDLERS: &str = r#"use inertia::Page;
use route::Cx;

use crate::{error::AppError, responses::home::Home, state::App};

pub async fn home(cx: Cx<App>) -> Page<Home> {
    inertia::render("home", Home { app_name: cx.cfg.app_name.clone() })
}

pub async fn health(cx: Cx<App>) -> Result<&'static str, AppError> {
    sqlx::query("select 1").execute(cx.db.pool()).await?;
    Ok("ok")
}
"#;

const COMMANDS: &str = r#"use std::sync::Arc;

use command::Commands;
use event::Events;
use queue::Queue;
use route::Routes;

use crate::state::App;

/// Every command the binary runs. `serve` runs when none is given.
pub fn register(
    routes: &Routes,
    sessions: session::PostgresStore,
    queue: Queue<queue::PostgresStore>,
    events: Arc<Events<App>>,
) -> Commands<App> {
    Commands::new()
        .default("serve", route::command::serve(routes))
        .add("route:list", route::command::list(routes))
        .add("session:prune", session::command::prune(sessions))
        .add("event:list", event::command::list(&events))
        .add("queue:retry", queue::command::retry(queue.clone()))
        .add("queue:work", queue::command::work(queue, move |w| crate::jobs::register(events.register_jobs(w))))
}
"#;

const HOME_PROPS: &str = r#"use serde::Serialize;

/// Props for `resources/js/pages/home.vue`.
#[derive(Serialize)]
pub struct Home {
    pub app_name: String,
}
"#;

const PACKAGE_JSON: &str = r#"{
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build"
  },
  "dependencies": {
    "@inertiajs/vue3": "^3.11.0",
    "vue": "^3.5.0"
  },
  "devDependencies": {
    "@vitejs/plugin-vue": "^6.0.0",
    "vite": "^8.4.0"
  }
}
"#;

const VITE_CONFIG: &str = r#"import fs from 'node:fs'
import vue from '@vitejs/plugin-vue'
import { defineConfig, type Plugin } from 'vite'

// While the dev server runs, public/hot holds its URL. The app's `vite` crate
// reads it per request and points asset tags at the dev server.
function hotFile(): Plugin {
  const remove = () => fs.rmSync('public/hot', { force: true })
  return {
    name: 'hot-file',
    configureServer(server) {
      server.httpServer?.once('listening', () => {
        fs.writeFileSync('public/hot', `http://localhost:${server.config.server.port}`)
      })
      process.once('exit', remove)
      for (const signal of ['SIGINT', 'SIGTERM'] as const) {
        process.once(signal, () => process.exit())
      }
    },
  }
}

export default defineConfig(({ command }) => ({
  base: command === 'build' ? '/build/' : '/',
  publicDir: false,
  plugins: [vue(), hotFile()],
  build: {
    manifest: true,
    outDir: 'public/build',
    rollupOptions: { input: 'resources/js/app.ts' },
  },
  server: { port: 5173, strictPort: true, origin: 'http://localhost:5173' },
}))
"#;

const APP_TS: &str = r#"import { createInertiaApp } from '@inertiajs/vue3'
import { createApp, h, type DefineComponent } from 'vue'

const pages = import.meta.glob<DefineComponent>('./pages/**/*.vue')

createInertiaApp({
  resolve: (page) => pages[`./pages/${page}.vue`](),
  setup({ el, App, props, plugin }) {
    createApp({ render: () => h(App, props) })
      .use(plugin)
      .mount(el)
  },
})
"#;

const HOME_VUE: &str = r#"<script setup lang="ts">
defineProps<{ app_name: string }>()
</script>

<template>
  <main>
    <h1>{{ app_name }}</h1>
    <p>It works.</p>
  </main>
</template>
"#;

const APP_HTML: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{name}</title>
    {{ head }}
  </head>
  <body>
    {{ inertia }}
  </body>
</html>
"#;

const JOBS: &str = r#"use queue::{PostgresStore, Worker};

use crate::state::App;

/// Every job `queue:work` runs. Add `.register::<my_job::MyJob>()` per job.
pub fn register(w: Worker<PostgresStore, App>) -> Worker<PostgresStore, App> {
    w
}
"#;

const EVENTS: &str = r#"use event::EventsBuilder;

use crate::state::App;

/// Every listener. Add `.on(my_event::on_my_event)` or
/// `.on_queued("name", my_event::on_my_event)` per listener.
pub fn register(b: EventsBuilder<App>) -> EventsBuilder<App> {
    b
}
"#;
