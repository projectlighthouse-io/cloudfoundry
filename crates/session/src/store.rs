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

#[cfg(feature = "postgres")]
mod postgres {
    use sqlx::PgPool;

    use super::*;

    /// Sessions in the `sessions` table (`art publish session`).
    #[derive(Clone)]
    pub struct PostgresStore(PgPool);

    impl PostgresStore {
        pub fn new(pool: PgPool) -> Self {
            Self(pool)
        }

        /// Delete expired sessions. Returns how many. Run from `session:prune`,
        /// not on the request path.
        pub async fn prune(&self) -> Result<u64, sqlx::Error> {
            let res = sqlx::query("delete from sessions where expires_at <= $1")
                .bind(crate::unix_now())
                .execute(&self.0)
                .await?;
            Ok(res.rows_affected())
        }
    }

    impl Store for PostgresStore {
        async fn load(&self, id: &SessionId) -> Result<Option<Record>, Error> {
            let row: Option<(Vec<u8>, i64)> = sqlx::query_as("select data, expires_at from sessions where id = $1")
                .bind(id.as_str())
                .fetch_optional(&self.0)
                .await?;
            Ok(row.map(|(data, expires_at)| Record { id: id.clone(), data, expires_at }))
        }

        async fn save(&self, r: &Record) -> Result<(), Error> {
            sqlx::query(
                "insert into sessions (id, data, expires_at) values ($1, $2, $3)
                 on conflict (id) do update set data = excluded.data, expires_at = excluded.expires_at",
            )
            .bind(r.id.as_str())
            .bind(&r.data)
            .bind(r.expires_at)
            .execute(&self.0)
            .await?;
            Ok(())
        }

        async fn delete(&self, id: &SessionId) -> Result<(), Error> {
            sqlx::query("delete from sessions where id = $1").bind(id.as_str()).execute(&self.0).await?;
            Ok(())
        }
    }
}

#[cfg(feature = "postgres")]
pub use postgres::PostgresStore;
