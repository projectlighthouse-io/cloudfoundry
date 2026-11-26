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
        .replace("{command}", &source.dep("command"));

    let files: &[(&str, &str)] = &[
        ("Cargo.toml", &cargo_toml),
        (".gitignore", "/target\n.env\n"),
        (".env.example", ENV),
        ("migrations/.gitkeep", ""),
        ("src/main.rs", MAIN),
        ("src/state.rs", STATE),
        ("src/config.rs", CONFIG),
        ("src/error.rs", ERROR),
        ("src/routes.rs", ROUTES),
        ("src/handlers/mod.rs", HANDLERS),
        ("src/commands/mod.rs", COMMANDS),
    ];
    for (file, contents) in files {
        let file = dir.join(file);
        fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&file, contents.replace("{name}", name)).map_err(|e| format!("{}: {e}", file.display()))?;
    }

    println!("Created {name}.\n\n  cd {name}\n  cp .env.example .env   # set DATABASE_URL\n  art migrate\n  art dev");
    Ok(())
}

const CARGO_TOML: &str = r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
command = { {command} }
db = { {db}, features = ["postgres"] }
route = { {route}, features = ["server", "command"] }

clap = { version = "4", features = ["derive"] }
dotenvy = "0.15"
serde = { version = "1", features = ["derive"] }
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "postgres", "macros", "migrate"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
"#;

const ENV: &str = "APP_NAME={name}\nDATABASE_URL=postgres://localhost/{name}\n";

const MAIN: &str = r#"mod commands;
mod config;
mod error;
mod handlers;
mod routes;
mod state;

use db::Db;
use route::Router;

use crate::{config::Config, error::AppError, state::{App, Primary}};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let cfg = Config::from_env()?;

    let db: Db<Primary> = Db::connect(&cfg.database_url).await?;

    let app = App::new(cfg, db);

    let router = routes::register(Router::new()).build(app.clone())?;

    commands::register(&router).run(app, std::env::args()).await;
    Ok(())
}
"#;

const STATE: &str = r#"use std::sync::Arc;

use db::Db;

use crate::config::Config;

db::handle!(pub Primary: Postgres);

/// Everything a handler or command needs. Cheap to clone.
#[derive(Clone)]
pub struct App {
    pub cfg: Arc<Config>,
    pub db: Db<Primary>,
}

impl App {
    pub fn new(cfg: Config, db: Db<Primary>) -> Self {
        Self { cfg: Arc::new(cfg), db }
    }
}
"#;

const CONFIG: &str = r#"use crate::error::AppError;

/// Typed config, read once from the environment (and `.env` if present).
pub struct Config {
    pub app_name: String,
    pub database_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self, AppError> {
        let _ = dotenvy::dotenv();
        Ok(Self {
            app_name: std::env::var("APP_NAME").unwrap_or_else(|_| "{name}".into()),
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
        eprintln!("{self}");
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

const HANDLERS: &str = r#"use route::Cx;

use crate::{error::AppError, state::App};

pub async fn home(cx: Cx<App>) -> String {
    format!("{} is running", cx.cfg.app_name)
}

pub async fn health(cx: Cx<App>) -> Result<&'static str, AppError> {
    sqlx::query("select 1").execute(cx.db.pool()).await?;
    Ok("ok")
}
"#;

const COMMANDS: &str = r#"use command::Commands;
use route::Routes;

use crate::state::App;

/// Every command the binary runs. `serve` runs when none is given.
pub fn register(routes: &Routes) -> Commands<App> {
    Commands::new()
        .default("serve", route::command::serve(routes))
        .add("route:list", route::command::list(routes))
}
"#;
