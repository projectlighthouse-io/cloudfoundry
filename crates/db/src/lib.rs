//! Typed pool handles and explicit transactions over `sqlx`.
//!
//! ```ignore
//! db::handle!(pub Primary: Postgres);
//!
//! let primary: Db<Primary> = Db::connect(&url).await?;
//! sqlx::query("select 1").execute(primary.pool()).await?;
//! primary.tx(async |tx| {
//!     sqlx::query("insert ...").execute(&mut **tx).await?;
//!     Ok::<_, sqlx::Error>(())
//! }).await?;
//! ```

pub use sqlx;

use std::{fmt, marker::PhantomData, sync::OnceLock, time::Duration};

use sqlx::{Database, Pool, Transaction, pool::PoolOptions};

/// A pool handle type. Declare one with [`handle!`].
pub trait Handle: Sized + Send + Sync + 'static {
    type Database: Database;
    const NAME: &'static str;
    #[doc(hidden)]
    fn __slot() -> &'static OnceLock<Db<Self>>;
}

/// Declare a pool handle: `db::handle!(pub Primary: Postgres);`.
/// The database is a `sqlx` driver type enabled by this crate's features.
#[macro_export]
macro_rules! handle {
    ($vis:vis $name:ident : $db:ident) => {
        $vis struct $name;

        impl $crate::Handle for $name {
            type Database = $crate::sqlx::$db;
            const NAME: &'static str = stringify!($name);
            fn __slot() -> &'static ::std::sync::OnceLock<$crate::Db<Self>> {
                static SLOT: ::std::sync::OnceLock<$crate::Db<$name>> = ::std::sync::OnceLock::new();
                &SLOT
            }
        }
    };
}

/// A connection pool for handle `H`. `Db<Analytics>` where `Db<Primary>` is
/// expected does not compile. Cheap to clone.
pub struct Db<H: Handle> {
    pool: Pool<H::Database>,
    _handle: PhantomData<fn() -> H>,
}

impl<H: Handle> Clone for Db<H> {
    fn clone(&self) -> Self {
        Self::from_pool(self.pool.clone())
    }
}

impl<H: Handle> fmt::Debug for Db<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Db<{}>", H::NAME)
    }
}

/// Pool settings, defaulting to sqlx's own. Deserializable with feature
/// `configured`; missing keys keep their defaults, unknown keys are an error:
///
/// ```toml
/// [db]
/// max_connections = 20
/// acquire_timeout = "5s"
/// max_lifetime = "never"
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "configured",
    derive(configured::serde::Deserialize, configured::serde::Serialize),
    serde(crate = "configured::serde", default, deny_unknown_fields)
)]
pub struct PoolConfig {
    pub max_connections: u32,
    /// Connections kept open even when idle. At most `max_connections`.
    pub min_connections: u32,
    /// How long `acquire` waits for a free connection before failing.
    #[cfg_attr(feature = "configured", serde(with = "configured::duration"))]
    pub acquire_timeout: Duration,
    /// Close connections idle this long. `None` (`"never"` in config): never.
    #[cfg_attr(feature = "configured", serde(with = "configured::duration_or_never"))]
    pub idle_timeout: Option<Duration>,
    /// Close connections this old. `None` (`"never"` in config): never.
    #[cfg_attr(feature = "configured", serde(with = "configured::duration_or_never"))]
    pub max_lifetime: Option<Duration>,
}

/// sqlx 0.8 defaults: 10 max, 0 min, 30s acquire, 10m idle, 30m lifetime.
impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 10,
            min_connections: 0,
            acquire_timeout: Duration::from_secs(30),
            idle_timeout: Some(Duration::from_secs(10 * 60)),
            max_lifetime: Some(Duration::from_secs(30 * 60)),
        }
    }
}

/// A [`PoolConfig`] no pool can satisfy.
#[derive(Debug)]
pub struct ConfigError(String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "db pool config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl PoolConfig {
    /// Run by [`Db::connect_with`].
    pub fn check(&self) -> Result<(), ConfigError> {
        let err = |m: &str| Err(ConfigError(m.into()));
        if self.max_connections == 0 {
            return err("max_connections must be at least 1");
        }
        if self.min_connections > self.max_connections {
            return err("min_connections must not exceed max_connections");
        }
        if self.acquire_timeout.is_zero() {
            return err("acquire_timeout must be above zero");
        }
        if self.idle_timeout.is_some_and(|d| d.is_zero()) || self.max_lifetime.is_some_and(|d| d.is_zero()) {
            return err("idle_timeout and max_lifetime must be above zero; omit them for never");
        }
        Ok(())
    }
}

impl<H: Handle> Db<H> {
    pub async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        Ok(Self::from_pool(Pool::connect(url).await?))
    }

    /// Connect with pool settings. An invalid `cfg` is
    /// `sqlx::Error::Configuration` and opens no connection.
    pub async fn connect_with(url: &str, cfg: &PoolConfig) -> Result<Self, sqlx::Error> {
        cfg.check().map_err(|e| sqlx::Error::Configuration(e.into()))?;
        let pool = PoolOptions::<H::Database>::new()
            .max_connections(cfg.max_connections)
            .min_connections(cfg.min_connections)
            .acquire_timeout(cfg.acquire_timeout)
            .idle_timeout(cfg.idle_timeout)
            .max_lifetime(cfg.max_lifetime)
            .connect(url)
            .await?;
        Ok(Self::from_pool(pool))
    }

    /// Wrap a pool built with `sqlx` pool options.
    pub fn from_pool(pool: Pool<H::Database>) -> Self {
        Self { pool, _handle: PhantomData }
    }

    pub fn pool(&self) -> &Pool<H::Database> {
        &self.pool
    }

    /// Run `f` in a transaction. Commits on `Ok`, rolls back on `Err`.
    pub async fn tx<T, E>(&self, f: impl AsyncFnOnce(&mut Transaction<'static, H::Database>) -> Result<T, E>) -> Result<T, E>
    where
        E: From<sqlx::Error>,
    {
        let mut tx = self.pool.begin().await?;
        match f(&mut tx).await {
            Ok(v) => {
                tx.commit().await?;
                Ok(v)
            }
            Err(e) => {
                // The caller's error matters more than a failed rollback; dropping
                // the connection rolls back anyway.
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }
}

/// Why a singleton call failed; it names the handle.
#[cfg(feature = "singleton")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SingletonError {
    /// `set_singleton` ran twice for this handle.
    AlreadySet(&'static str),
    /// `singleton()` ran before `set_singleton`.
    NotSet(&'static str),
}

#[cfg(feature = "singleton")]
impl fmt::Display for SingletonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadySet(h) => write!(f, "db singleton for `{h}` is already set"),
            Self::NotSet(h) => write!(f, "db singleton for `{h}` was never set; call Db::set_singleton at startup"),
        }
    }
}

#[cfg(feature = "singleton")]
impl std::error::Error for SingletonError {}

#[cfg(feature = "singleton")]
impl<H: Handle> Db<H> {
    /// Fill the process-wide slot for `H`. Call once at startup; a second call
    /// returns [`SingletonError::AlreadySet`] and keeps the first handle.
    pub fn set_singleton(db: Self) -> Result<(), SingletonError> {
        H::__slot().set(db).map_err(|_| SingletonError::AlreadySet(H::NAME))
    }

    /// The handle stored by `set_singleton`, or [`SingletonError::NotSet`].
    pub fn singleton() -> Result<&'static Self, SingletonError> {
        H::__slot().get().ok_or(SingletonError::NotSet(H::NAME))
    }
}
