//! `session:prune` for the `command` crate.

use crate::{Error, Prune};

#[derive(clap::Args)]
pub struct PruneArgs {}

/// `session:prune`: delete expired sessions.
pub fn prune<S: 'static, St: Prune>(store: St) -> impl AsyncFnOnce(&S, PruneArgs) -> Result<(), Error> + use<S, St> {
    async move |_: &S, _: PruneArgs| {
        let n = store.prune().await?;
        println!("pruned {n} expired sessions");
        Ok(())
    }
}
