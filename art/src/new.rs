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

pub fn run(name: &str, path: Option<PathBuf>, git: Option<String>, driver: &str) -> Result<(), String> {
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
        .replace("{sse}", &source.dep("sse"))
        .replace("{configured}", &source.dep("configured"));
    let flag = if driver == "postgres" { String::new() } else { format!(" --driver {driver}") };
    let agents = AGENTS.replace("{driver}", driver).replace("{flag}", &flag);
    let url = database_url(driver, name);
    let env_example = format!("APP_NAME={name}\nAPP_KEY=\nDATABASE_URL={url}\n");
    let env = format!("APP_NAME={name}\nAPP_KEY={}\nDATABASE_URL={url}\n", crate::generate_key()?);

    let mut files: Vec<(&str, &str)> = vec![
        ("Cargo.toml", &cargo_toml),
        ("AGENTS.md", &agents),
        // Claude Code reads CLAUDE.md; it imports AGENTS.md so both stay one file.
        ("CLAUDE.md", "@AGENTS.md\n"),
        (".gitignore", "/target\n.env\n/node_modules\n/public/build\n/public/hot\n/bootstrap/ssr\n/*.db*\n"),
        (".env.example", &env_example),
        (".env", &env),
        ("migrations/.gitkeep", ""),
        ("src/main.rs", MAIN),
        ("src/state.rs", STATE),
        ("src/config.rs", CONFIG),
        ("config/default.toml", DEFAULT_TOML),
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
        (".cargo/config.toml", CARGO_CONFIG),
        ("resources/views/app.html", APP_HTML),
        ("public/.gitkeep", ""),
        ("src/commands/mod.rs", COMMANDS),
    ];
    files.extend_from_slice(FRONTEND);
    for (file, contents) in &files {
        let file = dir.join(file);
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let contents = for_driver(&contents.replace("{name}", name), driver);
        fs::write(&file, contents).map_err(|e| format!("{}: {e}", file.display()))?;
    }

    println!(
        "Created {name}.\n\n  cd {name}\n  # check DATABASE_URL in .env\n  npm install\n  art publish session{flag}   # sessions table migration\n  art publish queue{flag}     # jobs tables migration\n  art migrate\n  art dev"
    );
    Ok(())
}

/// The frontend `art new` writes: entry points, the home page, the stylesheet,
/// and the layouts and components that published pages import.
const FRONTEND: &[(&str, &str)] = &[
    ("resources/js/app.ts", APP_TS),
    ("resources/js/ssr.ts", SSR_TS),
    ("resources/js/pages/home.vue", HOME_VUE),
    ("resources/js/types/Home.ts", HOME_TS),
    // What `db::Uuid<T>` exports (feature `ts`); here so published pages build before the first export.
    ("resources/js/types/Uuid.ts", UUID_TS),
    ("resources/css/app.css", include_str!("../templates/app.css")),
    ("resources/js/layouts/AppLayout.vue", include_str!("../templates/AppLayout.vue")),
    ("resources/js/layouts/AuthLayout.vue", include_str!("../templates/AuthLayout.vue")),
    ("resources/js/components/Field.vue", include_str!("../templates/Field.vue")),
    ("resources/js/components/UserMenu.vue", include_str!("../templates/UserMenu.vue")),
];

/// Instructions for coding agents (Codex, Cursor, Claude Code via CLAUDE.md).
/// `art publish` appends each module's notes.
const AGENTS: &str = r#"# {name}

A Rust web app on the toolkit crates (https://toolkit.projectlighthouse.io): Inertia + Vue
frontend, {driver} database, `art` CLI. Read this before changing code.

## Docs

The toolkit docs are written for agents too. Read the page for a crate before using it:

- Overview and every page as Markdown: https://toolkit.projectlighthouse.io/llms.txt
- One crate: https://toolkit.projectlighthouse.io/docs/crates/<crate>.md (modules such as
  `auth`, `password`, `socialite`: `/docs/modules/<module>.md`)
- Everything in one file: https://toolkit.projectlighthouse.io/llms-full.txt

The crates are beta (`0.x`) and come from git, not crates.io; `Cargo.lock` pins the commit.
Trust the docs over memory: these crates are not in most training data.

## Layout

| Path | What |
| --- | --- |
| `src/main.rs` | Wiring: config, database, layers (session, CSRF, auth, Inertia), routes, commands |
| `config/default.toml` | Settings, one section per crate (`[session]`, `[db]`, ...); `config/<APP_ENV>.toml` overrides. Secrets via `required_secret_env(NAME)`, never literals; `art config:show` prints them as `<redacted>` (`--print-secret-values` to reveal) |
| `src/routes.rs` | Every route, with names; `art route:list` prints them |
| `src/handlers/` | Async functions: typed inputs (`Cx<App>`, `Path`, ..., the body last), one output |
| `src/requests/` | Input structs with `#[derive(Validate)]` rules |
| `src/responses/` | Page props (`Serialize` + `ts_rs::TS`); types land in `resources/js/types/` |
| `src/actions/`, `src/jobs/`, `src/events/`, `src/commands/`, `src/middleware/` | Units of work, queued jobs, events, CLI commands, tower layers |
| `migrations/` | sqlx migrations |
| `resources/js/pages/` | Vue pages, rendered by Inertia |
| `resources/js/layouts/`, `resources/js/components/` | `AppLayout` (signed-in pages; `wide` for landing pages), `AuthLayout`, `Field`, `UserMenu` (add links there) |
| `resources/css/app.css` | Design tokens and classes: buttons, fields, page head (`phead`), label strips (`strip`), hairline rows (`ledger`) |

## Commands

| Command | Use |
| --- | --- |
| `art dev` | Run the app, Vite and the type export, restarting on change |
| `art make:handler <name>` (also `action`, `request`, `response`, `job`, `event`, `command`, `middleware`, `migration`) | Create one file and its `mod` line; prefer these to writing files by hand |
| `art make:page <path>` | Props struct, Vue page and its TypeScript type |
| `art publish <crate>{flag}` | Copy a module's handlers, migrations and pages into the app (`--with-ui` for Inertia pages) |
| `art migrate` / `art migrate:rollback` | Apply or revert migrations |
| `art route:list` | Every route, method and name |
| `art build` | Production build |

## Rules

- No framework: handlers are plain async functions, middleware is `tower` layers in `main.rs`,
  data access is `sqlx` queries in your code. There is no ORM, controller or template layer.
- Layer order matters: session, then CSRF, then auth, then Inertia (first added is outermost).
- Validate input with `Valid<Json<T>>` / `Valid<Form<T>>`. Request structs in `src/requests/` use
  `#[derive(Validate)]` with `#[validate(required, max = 255)]`-style attributes; own rules implement
  `validate::Rule`.
- Config lives in `config/*.toml`, read by `configured` into `src/config.rs`. Add a setting as a field there and a key in `config/default.toml`; read env vars only through `env(...)` forms, never `std::env::var` in app code.
- Refer to routes by name (`Redirect::to_route("dashboard")`), not by path.
- Files copied by `art publish` are app code: edit them freely; upgrading a crate never touches them.
- After changing Rust code, run `cargo check`; after changing page props, `art dev` re-exports the types.
"#;

const CARGO_TOML: &str = r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
action = { {action} }
command = { {command}, features = ["action"] }
configured = { {configured} }
db = { {db}, features = ["postgres", "configured", "ts"] }
route = { {route}, features = ["server", "command", "action"] }
validate = { {validate}, features = ["route"] }
csrf = { {csrf}, features = ["route", "configured"] }
securecookie = { {securecookie} }
session = { {session}, features = ["postgres", "route", "csrf", "command", "configured"] }
inertia = { {inertia}, features = ["validate", "ssr", "configured"] }
vite = { {vite} }
queue = { {queue}, features = ["postgres", "command", "configured"] }
event = { {event}, features = ["queue", "command"] }
sse = { {sse}, features = ["route"] }

clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "postgres", "macros", "migrate"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
tower = "0.5"
ts-rs = "12"
tower-http = { version = "0.6", features = ["fs"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
"#;

fn database_url(driver: &str, name: &str) -> String {
    match driver {
        "sqlite" => format!("sqlite://{name}.db?mode=rwc"),
        "mysql" => format!("mysql://root@localhost/{name}"),
        _ => format!("postgres://localhost/{name}"),
    }
}

/// Templates are written for Postgres; other drivers swap the few names
/// that differ (sqlx type, store types, Cargo features, the database URL).
fn for_driver(text: &str, driver: &str) -> String {
    let (db, store, feature) = match driver {
        "sqlite" => ("Sqlite", "SqliteStore", "\"sqlite\""),
        "mysql" => ("MySql", "MysqlStore", "\"mysql\""),
        _ => return text.to_owned(),
    };
    text.replace("handle!(pub Primary: Postgres)", &format!("handle!(pub Primary: {db})"))
        .replace("PostgresStore", store)
        .replace("\"postgres\"", feature)
}

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

use configured::{ExposeSecret, Loaded};

use crate::{config::Config, error::{AppError, config_error}, state::{App, Primary}};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let Loaded { value: cfg, listing } = Config::load()?;
    let codec = Codec::new(Keys::parse(cfg.app_key.expose_secret()).map_err(|e| AppError::Config(format!("config: app_key: {e}")))?);

    let vite = Vite::load("public", &["resources/js/app.ts"]).map_err(|e| AppError::Config(format!("vite: {e}")))?;
    let asset_version = vite.manifest_hash().to_owned();

    let db: Db<Primary> = Db::connect_with(&cfg.database_url, &cfg.db).await?;
    let sessions = PostgresStore::new(db.pool().clone());

    let queue = Queue::new(queue::PostgresStore::new(db.pool().clone()));
    let events = Arc::new(events::register(Events::builder().queue(&queue)).build());

    // Checked now so a bad [queue] fails at startup, not when `queue:work` starts.
    cfg.queue.check().map_err(config_error)?;
    let app = App::new(cfg, db, queue.clone(), events.clone());
    let c = &app.cfg;

    let pages = inertia::layer().config(c.inertia.clone()).map_err(config_error)?.version(&asset_version).head(move || vite.tags());

    let router = routes::register(Router::new())
        .layer(session::layer(sessions.clone(), codec).config(c.session.clone()).map_err(config_error)?)
        .layer(csrf::layer::<Session>().config(c.csrf.clone()))
        .layer(pages)
        .fallback(ServeDir::new("public"))
        .build(app.clone())?;

    let worker = app.cfg.queue.clone();
    commands::register(&router, sessions, queue, events, worker, listing).run(app, std::env::args()).await;
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
    // Read once the app pushes jobs or emits events.
    #[allow(dead_code)]
    pub queue: Queue<PostgresStore>,
    #[allow(dead_code)]
    pub events: Arc<Events<App>>,
}

impl App {
    pub fn new(cfg: Config, db: Db<Primary>, queue: Queue<PostgresStore>, events: Arc<Events<App>>) -> Self {
        Self { cfg: Arc::new(cfg), db, queue, events }
    }
}
"#;

/// `config/default.toml`. Commented keys show the defaults.
const DEFAULT_TOML: &str = r#"# Config for every environment; config/<APP_ENV>.toml overrides keys from here.
# env(NAME), env(NAME, default), required_env(NAME), secret_env(NAME) and
# required_secret_env(NAME) read environment variables (and .env); a missing
# required one stops startup. `config:show` prints the result with secrets
# as <redacted>; `config:show --print-secret-values` reveals them.
# Docs: https://toolkit.projectlighthouse.io/docs/crates/configured.md

app_name = "env(APP_NAME, {name})"
app_url = "env(APP_URL, http://localhost:3000)"   # no trailing /
app_key = "required_secret_env(APP_KEY)"
database_url = "required_env(DATABASE_URL)"

[session]
# cookie_name = "session"
# lifetime = "14d"
# same_site = "lax"   # strict | lax | none
# secure = true

[csrf]
# secure = true

[inertia]
root_template = "resources/views/app.html"
pages_dir = "resources/js/pages"
ssr_url = "env(INERTIA_SSR_URL)"
# ssr_timeout = "2s"
# encrypt_history = false

[queue]
# queues = ["default"]
# concurrency = 4
# visibility = "5m"
# poll = "1s"

[db]
# max_connections = 10
# min_connections = 0
# acquire_timeout = "30s"
# idle_timeout = "10m"   # or "never"
# max_lifetime = "30m"   # or "never"
"#;

const CONFIG: &str = r#"use configured::{Listing, Loaded, SecretString};

use crate::{error::{AppError, config_error}, state::App};

/// Typed config from `config/default.toml` (and `config/<APP_ENV>.toml`),
/// with `env(...)` values read from the environment and `.env`.
/// Sections are the crates' own `Config` types; missing keys keep defaults.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub app_name: String,
    /// Public base URL for links in mail and redirects, without a trailing `/`.
    // Read by the handlers `art publish auth` / `socialite` / `teams` add.
    #[allow(dead_code)]
    pub app_url: String,
    pub app_key: SecretString,
    pub database_url: String,

    #[serde(default)]
    pub session: session::Config,

    #[serde(default)]
    pub csrf: csrf::Config,

    #[serde(default)]
    pub inertia: inertia::Config,

    #[serde(default)]
    pub queue: queue::WorkerConfig,

    #[serde(default)]
    pub db: db::PoolConfig,
}

impl Config {
    /// Fails with every missing env var and bad key listed.
    pub fn load() -> Result<Loaded<Self>, AppError> {
        configured::load("config").map_err(config_error)
    }
}

#[derive(clap::Args)]
pub struct ShowArgs {
    /// Print secret values instead of `<redacted>`.
    #[arg(long)]
    pub print_secret_values: bool,
}

/// `config:show`: the keys loaded at startup. Secrets print as `<redacted>`
/// unless `--print-secret-values` is passed. Commands receive `&App`; this
/// one needs only the listing it captured.
pub fn show(listing: Listing) -> impl AsyncFnOnce(&App, ShowArgs) -> Result<(), AppError> {
    async move |_: &App, args: ShowArgs| {
        if args.print_secret_values {
            eprintln!("warning: printing secret values");
            print!("{}", listing.show_secret_values());
        } else {
            print!("{}", listing.show());
        }
        Ok(())
    }
}
"#;

const ERROR: &str = r#"use std::fmt;

use route::{Body, IntoResponse, http::{Response, StatusCode}};

pub enum AppError {
    Config(String),
    Db(sqlx::Error),
    Routes(route::BuildError),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(msg) => f.write_str(msg),
            Self::Db(e) => write!(f, "database: {e}"),
            Self::Routes(e) => write!(f, "routes: {e}"),
        }
    }
}

/// `main` prints errors with `Debug`: show the readable message.
impl fmt::Debug for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for AppError {}

/// A startup config failure. The crates' config errors already name their section.
pub fn config_error(e: impl fmt::Display) -> AppError {
    AppError::Config(e.to_string())
}

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
    worker: queue::WorkerConfig,
    listing: configured::Listing,
) -> Commands<App> {
    Commands::new()
        .default("serve", route::command::serve(routes))
        .add("route:list", route::command::list(routes))
        .add("config:show", crate::config::show(listing))
        .add("session:prune", session::command::prune(sessions))
        .add("event:list", event::command::list(&events))
        .add("queue:retry", queue::command::retry(queue.clone()))
        // `main` checked `worker`; `--queue` / `--concurrency` still override it.
        .add("queue:work", queue::command::work(queue, move |w| {
            crate::jobs::register(events.register_jobs(w)).config(worker).expect("[queue] was checked at startup")
        }))
}
"#;

const HOME_PROPS: &str = r#"use serde::Serialize;
use ts_rs::TS;

/// Props for `resources/js/pages/home.vue`.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct Home {
    pub app_name: String,
}
"#;

const PACKAGE_JSON: &str = r#"{
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build && vite build --ssr resources/js/ssr.ts",
    "ssr": "node bootstrap/ssr/ssr.js"
  },
  "dependencies": {
    "@inertiajs/vue3": "^3.11.0",
    "vue": "^3.5.0"
  },
  "devDependencies": {
    "@vitejs/plugin-vue": "^6.0.0",
    "typescript": "~5.9.3",
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

export default defineConfig(({ command, isSsrBuild }) => ({
  base: command === 'build' ? '/build/' : '/',
  publicDir: false,
  plugins: [vue(), hotFile()],
  // The SSR bundle (`vite build --ssr resources/js/ssr.ts`) runs in Node: `npm run ssr`.
  build: isSsrBuild
    ? { outDir: 'bootstrap/ssr' }
    : { manifest: true, outDir: 'public/build', rollupOptions: { input: 'resources/js/app.ts' } },
  server: { port: 5173, strictPort: true, origin: 'http://localhost:5173' },
}))
"#;

const APP_TS: &str = r#"import '../css/app.css'
import { createInertiaApp } from '@inertiajs/vue3'
import { createApp, createSSRApp, h, type DefineComponent } from 'vue'

const pages = import.meta.glob<DefineComponent>('./pages/**/*.vue')

createInertiaApp({
  resolve: (page) => pages[`./pages/${page}.vue`](),
  setup({ el, App, props, plugin }) {
    // Hydrate markup the SSR server rendered; otherwise render from scratch.
    const create = el?.hasAttribute('data-server-rendered') ? createSSRApp : createApp
    create({ render: () => h(App, props) })
      .use(plugin)
      .mount(el)
  },
})
"#;

const SSR_TS: &str = r#"import { createInertiaApp } from '@inertiajs/vue3'
import createServer from '@inertiajs/vue3/server'
import { createSSRApp, h, type DefineComponent } from 'vue'
import { renderToString } from 'vue/server-renderer'

const pages = import.meta.glob<DefineComponent>('./pages/**/*.vue')

// The app posts each first visit's page to http://127.0.0.1:13714/render
// when INERTIA_SSR_URL is set. Bound to localhost: not for the outside world.
createServer(
  (page) =>
    createInertiaApp({
      page,
      render: renderToString,
      resolve: (component) => pages[`./pages/${component}.vue`](),
      setup({ App, props, plugin }) {
        return createSSRApp({ render: () => h(App, props) }).use(plugin)
      },
    }),
  { host: '127.0.0.1' },
)
"#;

const HOME_VUE: &str = r#"<script setup lang="ts">
import AppLayout from '../layouts/AppLayout.vue'
import type { Home } from '../types/Home'

defineProps<Home>()
</script>

<template>
  <AppLayout :title="app_name" wide>
    <section class="hero">
      <div class="hero-tex" aria-hidden="true" />
      <div class="hero-in">
        <p class="eyebrow mono">{{ app_name }}</p>
        <h1>It works.</h1>
        <p class="lede">Edit <code>resources/js/pages/home.vue</code>, or publish a module's pages with <code>art publish &lt;crate&gt; --with-ui</code>.</p>
      </div>
    </section>
  </AppLayout>
</template>
"#;

const APP_HTML: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{name}</title>
    <link rel="preconnect" href="https://fonts.googleapis.com" />
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
    <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=Geist+Mono:wght@400;500&display=swap" />
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

const HOME_TS: &str = r#"// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.

/**
 * Props for `resources/js/pages/home.vue`.
 */
export type Home = { app_name: string, };
"#;

/// Same bytes as `crates/db/tests/uuid.ts`, which a db test checks against ts-rs output.
const UUID_TS: &str = r#"// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.

/**
 * A row's id outside the app: a UUIDv7 string, branded with the type it names.
 */
export type Uuid<T extends string> = string & { readonly __uuid: T };
"#;

/// `ts-rs` writes page prop types where the Vue pages import them from.
/// TypeScript (5.9; Vue's compiler cannot use 7 yet) resolves them at build.
const CARGO_CONFIG: &str = r#"[env]
TS_RS_EXPORT_DIR = { value = "resources/js/types", relative = true }
"#;

#[cfg(test)]
mod tests {
    use std::path::{Component, Path, PathBuf};

    /// `a/b/../c` as `a/c`.
    fn normalize(path: &Path) -> String {
        let mut out = PathBuf::new();
        for c in path.components() {
            match c {
                Component::ParentDir => {
                    out.pop();
                }
                Component::CurDir => {}
                c => out.push(c),
            }
        }
        out.to_string_lossy().into_owned()
    }

    #[test]
    fn frontend_imports_resolve_to_written_files() {
        let written: Vec<&str> = super::FRONTEND.iter().map(|(path, _)| *path).collect();
        for (path, source) in super::FRONTEND {
            for line in source.lines().filter(|l| l.trim_start().starts_with("import ")) {
                let Some(spec) = line.split(['\'', '"']).nth(1).filter(|s| s.starts_with('.')) else { continue };
                let target = normalize(&Path::new(path).parent().unwrap_or(Path::new("")).join(spec));
                let found = [target.clone(), format!("{target}.ts")].iter().any(|t| written.contains(&t.as_str()));
                assert!(found, "{path} imports {spec}, which art new does not write");
            }
        }
    }

    #[test]
    fn signed_in_pages_get_the_app_name_and_the_stylesheet() {
        let layout = super::FRONTEND.iter().find(|(p, _)| p.ends_with("AppLayout.vue")).map(|(_, s)| *s).unwrap_or_default();
        assert_eq!(layout.matches("{name}").count(), 2, "brand and footer");
        assert!(super::APP_TS.starts_with("import '../css/app.css'"));
    }
}
