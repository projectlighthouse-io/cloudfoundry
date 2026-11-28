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
    /// The map, even if a panicking caller poisoned the lock: each critical
    /// section is one plain map update, so the data is still consistent.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, Record>> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Store for MemoryStore {
    async fn load(&self, id: &SessionId) -> Result<Option<Record>, Error> {
        Ok(self.lock().get(id).cloned())
    }

    async fn save(&self, record: &Record) -> Result<(), Error> {
        self.lock().insert(record.id.clone(), record.clone());
        Ok(())
    }

    async fn delete(&self, id: &SessionId) -> Result<(), Error> {
        self.lock().remove(id);
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

/// Sessions in Redis or Valkey. Each session is one key that Redis expires
/// itself, so `prune` has nothing to do. Shared by every app instance that
/// points at the same server.
///
/// ```ignore
/// let client = redis::Client::open("redis://127.0.0.1/")?;
/// let store = session::RedisStore::new(client.get_connection_manager().await?).prefix("app:session:");
/// ```
#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisStore {
    conn: redis::aio::ConnectionManager,
    prefix: String,
}

#[cfg(feature = "redis")]
impl RedisStore {
    pub fn new(conn: redis::aio::ConnectionManager) -> Self {
        Self { conn, prefix: "session:".into() }
    }

    /// Key prefix, `session:` by default. Give each app its own when they
    /// share a server.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    fn key(&self, id: &SessionId) -> String {
        format!("{}{}", self.prefix, id.as_str())
    }
}

// The value is the expiry (8 bytes, big endian) followed by the data, so
// `load` returns `expires_at` without a second command.
#[cfg(feature = "redis")]
impl Store for RedisStore {
    async fn load(&self, id: &SessionId) -> Result<Option<Record>, Error> {
        let value: Option<Vec<u8>> = redis::cmd("GET").arg(self.key(id)).query_async(&mut self.conn.clone()).await?;
        Ok(value.and_then(|v| {
            let (head, data) = v.split_at_checked(8)?;
            let expires_at = i64::from_be_bytes(head.try_into().ok()?);
            Some(Record { id: id.clone(), data: data.to_vec(), expires_at })
        }))
    }

    async fn save(&self, r: &Record) -> Result<(), Error> {
        if r.expires_at <= crate::unix_now() {
            return self.delete(&r.id).await;
        }
        let mut value = r.expires_at.to_be_bytes().to_vec();
        value.extend_from_slice(&r.data);
        let () = redis::cmd("SET").arg(self.key(&r.id)).arg(value).arg("EXAT").arg(r.expires_at).query_async(&mut self.conn.clone()).await?;
        Ok(())
    }

    async fn delete(&self, id: &SessionId) -> Result<(), Error> {
        let _: i64 = redis::cmd("DEL").arg(self.key(id)).query_async(&mut self.conn.clone()).await?;
        Ok(())
    }
}

#[cfg(feature = "redis")]
impl Prune for RedisStore {
    /// Redis expires keys itself.
    async fn prune(&self) -> Result<u64, Error> {
        Ok(0)
    }
}
