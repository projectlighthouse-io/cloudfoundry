//! `session:prune` for the `command` crate.

use crate::PostgresStore;

#[derive(clap::Args)]
pub struct PruneArgs {}

/// `session:prune`: delete expired sessions.
pub fn prune<S: 'static>(store: PostgresStore) -> impl AsyncFnOnce(&S, PruneArgs) -> Result<(), sqlx::Error> + use<S> {
    async move |_: &S, _: PruneArgs| {
        let n = store.prune().await?;
        println!("pruned {n} expired sessions");
        Ok(())
    }
}
