#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
//! The same checks against every SQL store. SQLite runs always (in memory);
//! Postgres and MySQL run when SESSION_PG_URL / SESSION_MYSQL_URL are set.

#![cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]

use session::{Prune, Record, SessionId, Store};

fn id(n: u8) -> SessionId {
    serde_json::from_str(&format!("\"id-{n}\"")).unwrap()
}

async fn check<St: Store + Prune>(store: St) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let live = Record { id: id(1), data: b"live".to_vec(), expires_at: now + 3600 };
    let dead = Record { id: id(2), data: b"dead".to_vec(), expires_at: now - 10 };
    store.save(&live).await.unwrap();
    store.save(&dead).await.unwrap();

    store.save(&Record { data: b"updated".to_vec(), ..live.clone() }).await.unwrap();
    assert_eq!(store.load(&id(1)).await.unwrap().unwrap().data, b"updated", "upsert");

    assert_eq!(store.prune().await.unwrap(), 1, "only the expired one");
    assert!(store.load(&id(2)).await.unwrap().is_none());

    store.delete(&id(1)).await.unwrap();
    assert!(store.load(&id(1)).await.unwrap().is_none());
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/sqlite/create_sessions.up.sql")).execute(&pool).await.unwrap();
    check(session::SqliteStore::new(pool)).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres() {
    let Ok(url) = std::env::var("SESSION_PG_URL") else { return eprintln!("SESSION_PG_URL not set; skipping") };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql("drop table if exists sessions").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/create_sessions.up.sql")).execute(&pool).await.unwrap();
    check(session::PostgresStore::new(pool)).await;
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn mysql() {
    let Ok(url) = std::env::var("SESSION_MYSQL_URL") else { return eprintln!("SESSION_MYSQL_URL not set; skipping") };
    let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
    sqlx::raw_sql("drop table if exists sessions").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/mysql/create_sessions.up.sql")).execute(&pool).await.unwrap();
    check(session::MysqlStore::new(pool)).await;
}
