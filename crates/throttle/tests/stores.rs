#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
//! The same checks against every store. Memory and SQLite always run;
//! Postgres and MySQL run when THROTTLE_PG_URL / THROTTLE_MYSQL_URL are set.

use std::time::Duration;

use throttle::{FixedWindow, Limiter, MemoryStore, Store, unix_ms};

async fn check<St: Store>(store: St) {
    let now = unix_ms();
    assert_eq!(store.get("k", now).await.unwrap(), None);
    let n = store.update("k", now, |s| (s.is_none(), Some((b"one".to_vec(), now + 1000)))).await.unwrap();
    assert!(n, "a new key has no state");
    store.update("k", now, |s| { assert_eq!(s, Some(&b"one"[..])); ((), None) }).await.unwrap();
    assert_eq!(store.get("k", now).await.unwrap().as_deref(), Some(&b"one"[..]), "None leaves the state");
    assert_eq!(store.get("k", now + 1000).await.unwrap(), None, "expired");
    store.update("k", now + 1000, |s| { assert!(s.is_none(), "expired state is not passed"); ((), None) }).await.unwrap();

    store.update("live", now, |_| ((), Some((vec![1], now + 5000)))).await.unwrap();
    assert_eq!(store.prune(now + 2000).await.unwrap(), 1, "only the expired key");
    store.clear("live").await.unwrap();
    assert_eq!(store.get("live", now).await.unwrap(), None);

    // Concurrent hits on one key are all counted.
    let l = Limiter::new(store, FixedWindow::new(1000, Duration::from_secs(60)));
    let hits: Vec<_> = (0..20).map(|_| { let l = l.clone(); tokio::spawn(async move { l.hit("busy").await.unwrap() }) }).collect();
    for h in hits {
        assert!(h.await.unwrap().allowed);
    }
    // `check` reports what one more hit would leave.
    assert_eq!(l.check("busy").await.unwrap().remaining, 1000 - 21);
}

#[tokio::test(flavor = "multi_thread")]
async fn memory() {
    check(MemoryStore::new()).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite() {
    let dir = std::env::temp_dir().join(format!("throttle-{}.db", std::process::id()));
    let url = format!("sqlite://{}?mode=rwc", dir.display());
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(4).connect(&url).await.unwrap();
    sqlx::raw_sql("drop table if exists throttle").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/sqlite/create_throttle.up.sql")).execute(&pool).await.unwrap();
    check(throttle::SqliteStore::new(pool)).await;
    let _ = std::fs::remove_file(dir);
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres() {
    let Ok(url) = std::env::var("THROTTLE_PG_URL") else { return eprintln!("THROTTLE_PG_URL not set; skipping") };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql("drop table if exists throttle").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/create_throttle.up.sql")).execute(&pool).await.unwrap();
    check(throttle::PostgresStore::new(pool)).await;
}

#[cfg(feature = "mysql")]
#[tokio::test(flavor = "multi_thread")]
async fn mysql() {
    let Ok(url) = std::env::var("THROTTLE_MYSQL_URL") else { return eprintln!("THROTTLE_MYSQL_URL not set; skipping") };
    let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
    sqlx::raw_sql("drop table if exists throttle").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/mysql/create_throttle.up.sql")).execute(&pool).await.unwrap();
    check(throttle::MysqlStore::new(pool)).await;
}
