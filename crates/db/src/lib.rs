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

use std::{fmt, marker::PhantomData, sync::OnceLock};

use sqlx::{Database, Pool, Transaction};

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

impl<H: Handle> Db<H> {
    pub async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        Ok(Self::from_pool(Pool::connect(url).await?))
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

#[cfg(feature = "singleton")]
impl<H: Handle> Db<H> {
    /// Fill the process-wide slot for `H`. Call once at startup.
    /// Panics if it was already set.
    pub fn set_singleton(db: Self) {
        if H::__slot().set(db).is_err() {
            panic!("db singleton for `{}` is already set", H::NAME);
        }
    }

    /// The handle stored by `set_singleton`. Panics if it was never set.
    pub fn singleton() -> &'static Self {
        H::__slot().get().unwrap_or_else(|| panic!("db singleton for `{}` was never set", H::NAME))
    }
}
