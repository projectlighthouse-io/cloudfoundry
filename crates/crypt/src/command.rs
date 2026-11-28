//! `crypt:rotate` for the `command` crate.

use crate::{Encrypter, rotate::{Column, Error, Rotate}};

#[derive(clap::Args)]
pub struct RotateArgs {
    /// The table, e.g. `totp_secrets`.
    #[arg(long)]
    pub table: String,
    /// The encrypted column, e.g. `secret_encrypted`.
    #[arg(long)]
    pub column: String,
    /// An integer key column to page by.
    #[arg(long, default_value = "id")]
    pub id: String,
    /// What the values were sealed for; `{id}` is each row's id, e.g. `factor2/totp:{id}`.
    #[arg(long, default_value = "")]
    pub purpose: String,
    /// Count what would change, write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `crypt:rotate --table t --column c [--id id] [--purpose p] [--dry-run]`:
/// re-encrypt a column with the newest key, so the old key can be dropped
/// from `APP_KEY` (newest first, comma-separated). `setup` gets the app state
/// and returns the pool and the encrypter:
///
/// ```ignore
/// .add("crypt:rotate", crypt::command::rotate(|app: &App| {
///     Ok((app.db.pool().clone(), crypt::Encrypter::new(crypt::Keys::parse(app.cfg.app_key.expose_secret())?)))
/// }))
/// ```
pub fn rotate<S: 'static, P: Rotate + 'static>(
    setup: impl FnOnce(&S) -> Result<(P, Encrypter), Error> + 'static,
) -> impl AsyncFnOnce(&S, RotateArgs) -> Result<(), Error> {
    async move |app: &S, a: RotateArgs| {
        let (pool, crypt) = setup(app)?;
        let column = Column { table: &a.table, column: &a.column, id: &a.id, purpose: &a.purpose };
        let done = pool.rotate(&crypt, &column, a.dry_run).await?;
        let mode = if a.dry_run { " (dry run, nothing written)" } else { "" };
        println!("{}.{}: {done}{mode}", a.table, a.column);
        if done.failed > 0 {
            return Err(format!("{} values did not open: wrong --purpose, or sealed with a key no longer in APP_KEY", done.failed).into());
        }
        Ok(())
    }
}
