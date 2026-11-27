//! `art`: scaffolds apps, runs the dev loop, forwards everything else to the
//! app binary with `cargo run --`.

mod make;
mod new;
mod publish;

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    sync::mpsc,
    time::Duration,
};

use clap::Parser;
use notify::{EventKind, RecursiveMode, Watcher};

#[derive(Parser)]
#[command(name = "art", about = "Scaffold and run apps. Unknown commands go to the app binary.")]
enum Cli {
    /// Scaffold a new app.
    New {
        name: String,
        /// Depend on the toolkit crates at this local checkout.
        #[arg(long, conflicts_with = "git")]
        path: Option<PathBuf>,
        /// Depend on the toolkit crates from this git repository.
        #[arg(long)]
        git: Option<String>,
        /// Database: postgres, sqlite or mysql.
        #[arg(long, default_value = "postgres", value_parser = ["postgres", "sqlite", "mysql"])]
        driver: String,
    },
    /// Run the app, rebuilding and restarting on change.
    Dev,
    /// Run pending migrations and refresh the .sqlx query cache.
    Migrate,
    /// Revert the last migration.
    #[command(name = "migrate:rollback")]
    MigrateRollback,
    /// New reversible migration under migrations/.
    #[command(name = "make:migration")]
    MakeMigration { name: String },
    /// Handler function stub in src/handlers/.
    #[command(name = "make:handler")]
    MakeHandler { name: String },
    /// Action struct with input, output and `impl Action` in src/actions/.
    #[command(name = "make:action")]
    MakeAction { name: String },
    /// Props struct in src/responses/ plus its Vue page, e.g. `runs/show`.
    #[command(name = "make:page")]
    MakePage { path: String },
    /// View struct with a `From` stub in src/responses/.
    #[command(name = "make:response")]
    MakeResponse { name: String },
    /// Command function and its arguments in src/commands/.
    #[command(name = "make:command")]
    MakeCommand { name: String },
    /// Job struct with `impl Job` in src/jobs/.
    #[command(name = "make:job")]
    MakeJob { name: String },
    /// Event struct in src/events/.
    #[command(name = "make:event")]
    MakeEvent {
        name: String,
        /// Add a listener function.
        #[arg(long)]
        listener: bool,
    },
    /// Run the Inertia SSR server built by `art build` (`npm run ssr`).
    Ssr,
    /// Vite production build, then `cargo build --release`.
    Build,
    /// Tower middleware stub in src/middleware/.
    #[command(name = "make:middleware")]
    MakeMiddleware { name: String },
    /// Copy a dependency's stub files into the app.
    Publish {
        package: String,
        /// Database the stubs are for: `sqlite` or `mysql`. Default: Postgres.
        #[arg(long)]
        driver: Option<String>,
        /// Also publish the package's UI: Inertia handlers and Vue pages.
        #[arg(long)]
        with_ui: bool,
        /// Show what would be copied.
        #[arg(long)]
        list: bool,
        /// Overwrite existing files.
        #[arg(long)]
        force: bool,
    },
    /// Print a fresh APP_KEY.
    #[command(name = "key:generate")]
    KeyGenerate,
    /// Input struct with `impl Validate` in src/requests/.
    #[command(name = "make:request")]
    MakeRequest { name: String },
    #[command(external_subcommand)]
    App(Vec<String>),
}

fn main() -> ExitCode {
    let result = match Cli::parse() {
        Cli::New { name, path, git, driver } => new::run(&name, path, git, &driver),
        Cli::Dev => dev(),
        Cli::Migrate => sqlx(&["migrate", "run"]).and_then(|()| refresh_query_cache()),
        Cli::MigrateRollback => sqlx(&["migrate", "revert"]),
        Cli::MakeMigration { name } => sqlx(&["migrate", "add", "-r", &name]),
        Cli::MakeHandler { name } => make::run(make::Kind::Handler, &name),
        Cli::MakeAction { name } => make::run(make::Kind::Action, &name),
        Cli::MakeRequest { name } => make::run(make::Kind::Request, &name),
        Cli::MakeMiddleware { name } => make::run(make::Kind::Middleware, &name),
        Cli::MakePage { path } => make::run(make::Kind::Page, &path),
        Cli::MakeResponse { name } => make::run(make::Kind::Response, &name),
        Cli::Build => build(),
        Cli::Ssr => status(Command::new("npm").args(["run", "ssr"]), NPM_MISSING),
        Cli::MakeJob { name } => make::run(make::Kind::Job, &name),
        Cli::MakeCommand { name } => make::run(make::Kind::Command, &name),
        Cli::MakeEvent { name, listener } => make::run(make::Kind::Event { listener }, &name),
        Cli::Publish { package, driver, with_ui, list, force } => publish::run(&package, driver.as_deref(), with_ui, list, force),
        Cli::KeyGenerate => generate_key().map(|k| println!("{k}")),
        Cli::App(args) => return forward(&args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("art: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 32 random bytes, base64: the format `securecookie::Keys::parse` reads.
fn generate_key() -> Result<String, String> {
    use base64::Engine;
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).map_err(|e| format!("the OS random source failed: {e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(k))
}

const SQLX_INSTALL: &str =
    "sqlx-cli is not installed (or lacks this database). Install it with: cargo install sqlx-cli --no-default-features --features rustls,postgres,sqlite,mysql";

fn status(cmd: &mut Command, missing: &str) -> Result<(), String> {
    match cmd.status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{:?} failed with {s}", cmd.get_program())),
        Err(e) if e.kind() == ErrorKind::NotFound => Err(missing.to_owned()),
        Err(e) => Err(e.to_string()),
    }
}

fn sqlx(args: &[&str]) -> Result<(), String> {
    status(Command::new("sqlx").args(args), SQLX_INSTALL)
}

fn refresh_query_cache() -> Result<(), String> {
    status(Command::new("cargo").args(["sqlx", "prepare"]), SQLX_INSTALL)
}

fn forward(args: &[String]) -> ExitCode {
    match Command::new("cargo").args(["run", "--quiet", "--"]).args(args).status() {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!("art: cargo: {e}");
            ExitCode::FAILURE
        }
    }
}

const NPM_MISSING: &str = "npm is not installed; install Node.js to build the frontend";

/// Regenerate TypeScript types for page props (`ts-rs`, exported by its
/// generated tests into `TS_RS_EXPORT_DIR`). Skipped in apps without it.
fn export_types() -> bool {
    let configured = fs::read_to_string(".cargo/config.toml").is_ok_and(|c| c.contains("TS_RS_EXPORT_DIR"));
    if !configured {
        return true;
    }
    let ok = Command::new("cargo").args(["test", "--quiet", "export_bindings"]).status().is_ok_and(|s| s.success());
    if !ok {
        eprintln!("art: exporting TypeScript types failed; see the output above");
    }
    ok
}

fn build() -> Result<(), String> {
    if !export_types() {
        return Err("TypeScript type export failed".into());
    }
    if Path::new("package.json").exists() {
        status(Command::new("npm").args(["run", "build"]), NPM_MISSING)?;
    }
    status(Command::new("cargo").args(["build", "--release"]), "cargo not found")
}

/// Restart `cargo run -- serve` whenever src/, migrations/ or Cargo.toml change.
/// `cargo run` execs the app on Unix, so killing the child stops the app too.
fn dev() -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx).map_err(|e| e.to_string())?;
    for p in ["src", "migrations", "Cargo.toml"].map(Path::new).iter().filter(|p| p.exists()) {
        watcher.watch(p, RecursiveMode::Recursive).map_err(|e| format!("watch {}: {e}", p.display()))?;
    }

    // Vite runs once for the whole session; it hot-reloads the frontend itself.
    // Ctrl-C reaches it too: it shares the terminal's process group.
    let _vite = if Path::new("package.json").exists() {
        Some(Command::new("npm").args(["run", "dev"]).spawn().map_err(|e| format!("npm: {e}"))?)
    } else {
        None
    };

    export_types();
    loop {
        let mut app = Command::new("cargo").args(["run", "--", "serve"]).spawn().map_err(|e| format!("cargo: {e}"))?;

        let mut changed = vec![];
        loop {
            match rx.recv() {
                Ok(Ok(ev)) if matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)) => {
                    changed.extend(ev.paths);
                    break;
                }
                Ok(_) => {}
                Err(_) => return Err("file watcher stopped".into()),
            }
        }
        // Editors write in bursts; wait for quiet before rebuilding.
        while let Ok(ev) = rx.recv_timeout(Duration::from_millis(200)) {
            changed.extend(ev.into_iter().flat_map(|e| e.paths));
        }

        println!("art: change detected, restarting");
        let _ = app.kill();
        let _ = app.wait();
        if changed.iter().any(|p| p.components().any(|c| c.as_os_str() == "responses")) {
            export_types();
        }
    }
}
