//! `throttle:prune` for the `command` crate.

use crate::{Error, Store, unix_ms};

#[derive(clap::Args)]
pub struct PruneArgs {}

/// `throttle:prune`: delete expired limits.
pub fn prune<S: 'static, St: Store>(store: St) -> impl AsyncFnOnce(&S, PruneArgs) -> Result<(), Error> + use<S, St> {
    async move |_: &S, _: PruneArgs| {
        let n = store.prune(unix_ms()).await?;
        println!("pruned {n} expired throttle keys");
        Ok(())
    }
}
