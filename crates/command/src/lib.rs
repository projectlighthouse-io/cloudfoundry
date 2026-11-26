//! Register and dispatch commands in the app binary.
//!
//! ```ignore
//! #[derive(clap::Args)]
//! struct PruneArgs {
//!     #[arg(long, default_value = "30")]
//!     days: u32,
//! }
//!
//! async fn prune_runs(app: &App, args: PruneArgs) -> Result<(), AppError> { /* ... */ }
//!
//! Commands::new()
//!     .default("serve", serve)
//!     .add("runs:prune", prune_runs)
//!     .run(app, std::env::args())
//!     .await;
//! ```

pub use clap;

use std::{ffi::OsString, fmt::Display, future::Future, pin::Pin};

use clap::{ArgMatches, Args, FromArgMatches};

type Run<S> = Box<dyn for<'a> FnOnce(&'a S, ArgMatches) -> Pin<Box<dyn Future<Output = Result<(), String>> + 'a>>>;

struct Entry<S> {
    name: &'static str,
    cmd: clap::Command,
    run: Run<S>,
}

/// A registry of named commands. Built at startup, consumed by `run`.
pub struct Commands<S> {
    entries: Vec<Entry<S>>,
    default: Option<&'static str>,
}

impl<S> Default for Commands<S> {
    fn default() -> Self {
        Self { entries: vec![], default: None }
    }
}

impl<S> Commands<S> {
    pub fn new() -> Self {
        <Self as Default>::default()
    }

    /// Register `name` (`group:verb`). `f` is an async fn taking the app and
    /// its `clap::Args`. Panics on a duplicate name: that is a startup bug.
    pub fn add<A, F, E>(mut self, name: &'static str, f: F) -> Self
    where
        A: Args + FromArgMatches + 'static,
        F: AsyncFnOnce(&S, A) -> Result<(), E> + 'static,
        E: Display,
    {
        assert!(self.entries.iter().all(|e| e.name != name), "command `{name}` is registered twice");
        let run: Run<S> = Box::new(move |state, matches| {
            Box::pin(async move {
                let args = A::from_arg_matches(&matches).map_err(|e| e.to_string())?;
                f(state, args).await.map_err(|e| e.to_string())
            })
        });
        self.entries.push(Entry { name, cmd: A::augment_args(clap::Command::new(name)), run });
        self
    }

    /// Register `name` and run it when no command is given.
    pub fn default<A, F, E>(mut self, name: &'static str, f: F) -> Self
    where
        A: Args + FromArgMatches + 'static,
        F: AsyncFnOnce(&S, A) -> Result<(), E> + 'static,
        E: Display,
    {
        self = self.add(name, f);
        self.default = Some(name);
        self
    }

    /// Parse `args` (including the binary name) and run the chosen command.
    /// Help and usage errors exit through clap; a command error prints to
    /// stderr and exits 1.
    pub async fn run<I, T>(self, state: S, args: I)
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        match self.dispatch(&state, args).await {
            Ok(()) => {}
            Err(Failure::Usage(e)) => e.exit(),
            Err(Failure::Command(msg)) => {
                eprintln!("error: {msg}");
                std::process::exit(1);
            }
        }
    }

    async fn dispatch<I, T>(self, state: &S, args: I) -> Result<(), Failure>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let root = clap::Command::new("app")
            .subcommand_required(self.default.is_none())
            .subcommands(self.entries.iter().map(|e| e.cmd.clone()));
        let matches = root.try_get_matches_from(args).map_err(Failure::Usage)?;

        let (name, sub) = match matches.subcommand() {
            Some((name, sub)) => (name.to_owned(), sub.clone()),
            None => {
                let name = self.default.expect("clap requires a subcommand when there is no default");
                let cmd = &self.entries.iter().find(|e| e.name == name).unwrap().cmd;
                (name.to_owned(), cmd.clone().try_get_matches_from([name]).map_err(Failure::Usage)?)
            }
        };
        let entry = self.entries.into_iter().find(|e| e.name == name).expect("clap only matches registered names");
        (entry.run)(state, sub).await.map_err(Failure::Command)
    }
}

enum Failure {
    Usage(clap::Error),
    Command(String),
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, pin::pin, task::{Context, Poll, Waker}};

    use super::*;

    #[derive(clap::Args)]
    struct SetArgs {
        #[arg(long, default_value = "7")]
        n: u32,
    }

    async fn set(state: &Cell<u32>, args: SetArgs) -> Result<(), String> {
        state.set(args.n);
        Ok(())
    }

    async fn fail(_: &Cell<u32>, _: SetArgs) -> Result<(), String> {
        Err("boom".into())
    }

    // The commands under test never await anything pending, so one poll is enough.
    fn block_on<F: Future>(f: F) -> F::Output {
        match pin!(f).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("test future was pending"),
        }
    }

    fn commands() -> Commands<Cell<u32>> {
        Commands::new().default("x:set", set).add("x:fail", fail)
    }

    #[test]
    fn dispatches() {
        let state = Cell::new(0);
        assert!(block_on(commands().dispatch(&state, ["app", "x:set", "--n", "3"])).is_ok());
        assert_eq!(state.get(), 3);

        assert!(block_on(commands().dispatch(&state, ["app"])).is_ok(), "default runs with no command");
        assert_eq!(state.get(), 7);

        assert!(matches!(block_on(commands().dispatch(&state, ["app", "x:fail"])), Err(Failure::Command(m)) if m == "boom"));
        assert!(matches!(block_on(commands().dispatch(&state, ["app", "nope"])), Err(Failure::Usage(_))));
    }
}
