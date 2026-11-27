//! SQL stores. Publish the table with `art publish throttle [--driver sqlite|mysql]`.
//!
//! `update` runs in a transaction: insert an expired placeholder row if the
//! key has none, lock the row (`for update`; SQLite's write lock is taken by
//! the insert), run the algorithm, write the result. Two servers hitting the
//! same key queue on the row lock, so no hit is lost.

use crate::{Error, Store};

macro_rules! sql_store {
    ($(#[$m:meta])* $name:ident, $pool:ty,
     get: $get:literal, placeholder: $placeholder:literal, lock: $lock:literal,
     write: $write:literal, clear: $clear:literal, prune: $prune:literal) => {
        $(#[$m])*
        #[derive(Clone)]
        pub struct $name($pool);

        impl $name {
            pub fn new(pool: $pool) -> Self {
                Self(pool)
            }
        }

        impl Store for $name {
            async fn get(&self, key: &str, now: i64) -> Result<Option<Vec<u8>>, Error> {
                let row: Option<(Vec<u8>,)> = sqlx::query_as($get).bind(key).bind(now).fetch_optional(&self.0).await?;
                Ok(row.map(|(s,)| s))
            }

            async fn update<F, R>(&self, key: &str, now: i64, f: F) -> Result<R, Error>
            where
                F: FnOnce(Option<&[u8]>) -> (R, Option<(Vec<u8>, i64)>) + Send,
                R: Send,
            {
                let mut tx = self.0.begin().await?;
                sqlx::query($placeholder).bind(key).execute(&mut *tx).await?;
                let (state, expires_at): (Vec<u8>, i64) = sqlx::query_as($lock).bind(key).fetch_one(&mut *tx).await?;
                let (out, new) = f((expires_at > now).then_some(state.as_slice()));
                if let Some((state, expires_at)) = new {
                    sqlx::query($write).bind(state).bind(expires_at).bind(key).execute(&mut *tx).await?;
                }
                tx.commit().await?;
                Ok(out)
            }

            async fn clear(&self, key: &str) -> Result<(), Error> {
                sqlx::query($clear).bind(key).execute(&self.0).await?;
                Ok(())
            }

            async fn prune(&self, now: i64) -> Result<u64, Error> {
                Ok(sqlx::query($prune).bind(now).execute(&self.0).await?.rows_affected())
            }
        }
    };
}

#[cfg(feature = "postgres")]
sql_store!(
    /// Limits in Postgres (`art publish throttle`).
    PostgresStore, sqlx::PgPool,
    get: "select state from throttle where id = $1 and expires_at > $2",
    placeholder: "insert into throttle (id, state, expires_at) values ($1, ''::bytea, 0) on conflict (id) do nothing",
    lock: "select state, expires_at from throttle where id = $1 for update",
    write: "update throttle set state = $1, expires_at = $2 where id = $3",
    clear: "delete from throttle where id = $1",
    prune: "delete from throttle where expires_at <= $1"
);

#[cfg(feature = "sqlite")]
sql_store!(
    /// Limits in SQLite (`art publish throttle --driver sqlite`).
    SqliteStore, sqlx::SqlitePool,
    get: "select state from throttle where id = ? and expires_at > ?",
    placeholder: "insert into throttle (id, state, expires_at) values (?, x'', 0) on conflict (id) do nothing",
    lock: "select state, expires_at from throttle where id = ?",
    write: "update throttle set state = ?, expires_at = ? where id = ?",
    clear: "delete from throttle where id = ?",
    prune: "delete from throttle where expires_at <= ?"
);

#[cfg(feature = "mysql")]
sql_store!(
    /// Limits in MySQL 8 or MariaDB (`art publish throttle --driver mysql`).
    MysqlStore, sqlx::MySqlPool,
    get: "select state from throttle where id = ? and expires_at > ?",
    placeholder: "insert into throttle (id, state, expires_at) values (?, x'', 0) on duplicate key update id = id",
    lock: "select state, expires_at from throttle where id = ? for update",
    write: "update throttle set state = ?, expires_at = ? where id = ?",
    clear: "delete from throttle where id = ?",
    prune: "delete from throttle where expires_at <= ?"
);
