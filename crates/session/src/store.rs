use std::{collections::HashMap, future::Future, sync::Mutex};

use crate::{Error, SessionId};

/// One saved session. `data` is opaque to the store.
#[derive(Clone)]
pub struct Record {
    pub id: SessionId,
    pub data: Vec<u8>,
    /// Unix seconds.
    pub expires_at: i64,
}

/// Where sessions live. A driver is three queries.
pub trait Store: Send + Sync + 'static {
    /// The record, if it exists. Expired records may be returned; the layer
    /// ignores them.
    fn load(&self, id: &SessionId) -> impl Future<Output = Result<Option<Record>, Error>> + Send;
    fn save(&self, record: &Record) -> impl Future<Output = Result<(), Error>> + Send;
    fn delete(&self, id: &SessionId) -> impl Future<Output = Result<(), Error>> + Send;
}

/// In-process store, for tests and local experiments. Lost on restart.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<SessionId, Record>>);

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Store for MemoryStore {
    async fn load(&self, id: &SessionId) -> Result<Option<Record>, Error> {
        Ok(self.0.lock().unwrap().get(id).cloned())
    }

    async fn save(&self, record: &Record) -> Result<(), Error> {
        self.0.lock().unwrap().insert(record.id.clone(), record.clone());
        Ok(())
    }

    async fn delete(&self, id: &SessionId) -> Result<(), Error> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }
}

/// Stores that can delete expired sessions, for `session:prune`.
pub trait Prune: Store {
    /// Delete expired sessions; returns how many.
    fn prune(&self) -> impl Future<Output = Result<u64, Error>> + Send;
}

/// One store per SQL driver. Only the SQL text differs.
#[allow(unused_macros)]
macro_rules! sql_store {
    ($(#[$m:meta])* $name:ident, $pool:ty, load: $load:literal, save: $save:literal, delete: $delete:literal, prune: $prune:literal) => {
        $(#[$m])*
        #[derive(Clone)]
        pub struct $name($pool);

        impl $name {
            pub fn new(pool: $pool) -> Self {
                Self(pool)
            }
        }

        impl Store for $name {
            async fn load(&self, id: &SessionId) -> Result<Option<Record>, Error> {
                let row: Option<(Vec<u8>, i64)> = sqlx::query_as($load).bind(id.as_str()).fetch_optional(&self.0).await?;
                Ok(row.map(|(data, expires_at)| Record { id: id.clone(), data, expires_at }))
            }

            async fn save(&self, r: &Record) -> Result<(), Error> {
                sqlx::query($save).bind(r.id.as_str()).bind(&r.data).bind(r.expires_at).execute(&self.0).await?;
                Ok(())
            }

            async fn delete(&self, id: &SessionId) -> Result<(), Error> {
                sqlx::query($delete).bind(id.as_str()).execute(&self.0).await?;
                Ok(())
            }
        }

        impl Prune for $name {
            async fn prune(&self) -> Result<u64, Error> {
                Ok(sqlx::query($prune).bind(crate::unix_now()).execute(&self.0).await?.rows_affected())
            }
        }
    };
}

#[cfg(feature = "postgres")]
sql_store!(
    /// Sessions in Postgres (`art publish session`).
    PostgresStore, sqlx::PgPool,
    load: "select data, expires_at from sessions where id = $1",
    save: "insert into sessions (id, data, expires_at) values ($1, $2, $3)
           on conflict (id) do update set data = excluded.data, expires_at = excluded.expires_at",
    delete: "delete from sessions where id = $1",
    prune: "delete from sessions where expires_at <= $1"
);

#[cfg(feature = "sqlite")]
sql_store!(
    /// Sessions in SQLite (`art publish session --driver sqlite`).
    SqliteStore, sqlx::SqlitePool,
    load: "select data, expires_at from sessions where id = ?",
    save: "insert into sessions (id, data, expires_at) values (?, ?, ?)
           on conflict (id) do update set data = excluded.data, expires_at = excluded.expires_at",
    delete: "delete from sessions where id = ?",
    prune: "delete from sessions where expires_at <= ?"
);

#[cfg(feature = "mysql")]
sql_store!(
    /// Sessions in MySQL 8 or MariaDB (`art publish session --driver mysql`).
    MysqlStore, sqlx::MySqlPool,
    load: "select data, expires_at from sessions where id = ?",
    save: "insert into sessions (id, data, expires_at) values (?, ?, ?)
           on duplicate key update data = values(data), expires_at = values(expires_at)",
    delete: "delete from sessions where id = ?",
    prune: "delete from sessions where expires_at <= ?"
);
