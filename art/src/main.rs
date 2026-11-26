//! `art`: scaffolds apps, runs the dev loop, forwards everything else to the
//! app binary with `cargo run --`.

mod new;

use std::{
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
    #[command(external_subcommand)]
    App(Vec<String>),
}

fn main() -> ExitCode {
    let result = match Cli::parse() {
        Cli::New { name, path, git } => new::run(&name, path, git),
        Cli::Dev => dev(),
        Cli::Migrate => sqlx(&["migrate", "run"]).and_then(|()| refresh_query_cache()),
        Cli::MigrateRollback => sqlx(&["migrate", "revert"]),
        Cli::MakeMigration { name } => sqlx(&["migrate", "add", "-r", &name]),
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

const SQLX_INSTALL: &str =
    "sqlx-cli is not installed. Install it with: cargo install sqlx-cli --no-default-features --features rustls,postgres";

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

/// Restart `cargo run -- serve` whenever src/, migrations/ or Cargo.toml change.
/// `cargo run` execs the app on Unix, so killing the child stops the app too.
fn dev() -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx).map_err(|e| e.to_string())?;
    for p in ["src", "migrations", "Cargo.toml"].map(Path::new).iter().filter(|p| p.exists()) {
        watcher.watch(p, RecursiveMode::Recursive).map_err(|e| format!("watch {}: {e}", p.display()))?;
    }

    loop {
        let mut app = Command::new("cargo").args(["run", "--", "serve"]).spawn().map_err(|e| format!("cargo: {e}"))?;

        loop {
            match rx.recv() {
                Ok(Ok(ev)) if matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)) => break,
                Ok(_) => {}
                Err(_) => return Err("file watcher stopped".into()),
            }
        }
        // Editors write in bursts; wait for quiet before rebuilding.
        while rx.recv_timeout(Duration::from_millis(200)).is_ok() {}

        println!("art: change detected, restarting");
        let _ = app.kill();
        let _ = app.wait();
    }
}
