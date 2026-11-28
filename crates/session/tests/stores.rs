#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
//! The same checks against every SQL store. SQLite runs always (in memory);
//! Postgres, MySQL and Redis run when SESSION_PG_URL / SESSION_MYSQL_URL /
//! SESSION_REDIS_URL are set.

#![cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql", feature = "redis"))]

use session::{Device, Devices, Prune, Record, SessionId, Store};

fn id(n: u8) -> SessionId {
    serde_json::from_str(&format!("\"id-{n}\"")).unwrap()
}

#[allow(dead_code, reason = "unused when only redis is enabled")]
async fn check<St: Store + Prune>(store: St) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let live = Record { id: id(1), data: b"live".to_vec(), expires_at: now + 3600, user_id: None, device: None };
    let dead = Record { id: id(2), data: b"dead".to_vec(), expires_at: now - 10, user_id: None, device: None };
    store.save(&live).await.unwrap();
    store.save(&dead).await.unwrap();

    store.save(&Record { data: b"updated".to_vec(), ..live.clone() }).await.unwrap();
    assert_eq!(store.load(&id(1)).await.unwrap().unwrap().data, b"updated", "upsert");

    assert_eq!(store.prune().await.unwrap(), 1, "only the expired one");
    assert!(store.load(&id(2)).await.unwrap().is_none());

    store.delete(&id(1)).await.unwrap();
    assert!(store.load(&id(1)).await.unwrap().is_none());
}

/// A user's sessions: listed most recent first, unexpired only, revoked one by one or all but one.
#[allow(dead_code, reason = "unused when only redis is enabled")]
async fn check_devices<St: Devices>(store: St) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let device = |seen: i64| Device { public_id: uuid::Uuid::new_v4(), user_agent: Some("Firefox".into()), ip: Some("10.0.0.1".into()), last_seen_at: seen };
    let (a, b, c, old) = (device(now - 50), device(now - 10), device(now), device(now - 5));
    for (n, user, d, expires_at) in [(1, Some(1), &a, now + 60), (2, Some(1), &b, now + 60), (3, Some(2), &c, now + 60), (4, Some(1), &old, now - 1)] {
        store.save(&Record { id: id(n), data: vec![], expires_at, user_id: user, device: Some(d.clone()) }).await.unwrap();
    }
    store.save(&Record { id: id(5), data: vec![], expires_at: now + 60, user_id: None, device: None }).await.unwrap();

    assert_eq!(store.devices(1).await.unwrap(), [b.clone(), a.clone()], "newest first, expired left out");
    assert!(!store.revoke(2, a.public_id).await.unwrap(), "not theirs");
    assert!(store.revoke(1, a.public_id).await.unwrap());
    assert!(store.load(&id(1)).await.unwrap().is_none(), "revoked: the session is gone");

    store.save(&Record { id: id(6), data: vec![], expires_at: now + 60, user_id: Some(1), device: Some(device(now)) }).await.unwrap();
    assert_eq!(store.revoke_others(1, b.public_id).await.unwrap(), 2, "the other one and the expired one");
    assert_eq!(store.devices(1).await.unwrap(), [b]);
    assert_eq!(store.devices(2).await.unwrap(), [c], "other users keep theirs");
    assert!(store.load(&id(5)).await.unwrap().is_some(), "anonymous sessions stay");
}

#[tokio::test]
async fn memory_devices() {
    check_devices(session::MemoryStore::new()).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/sqlite/create_sessions.up.sql")).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/sqlite/add_session_devices.up.sql")).execute(&pool).await.unwrap();
    check(session::SqliteStore::new(pool.clone())).await;
    check_devices(session::SqliteStore::new(pool)).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres() {
    let Ok(url) = std::env::var("SESSION_PG_URL") else { return eprintln!("SESSION_PG_URL not set; skipping") };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql("drop table if exists sessions").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/create_sessions.up.sql")).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/add_session_devices.up.sql")).execute(&pool).await.unwrap();
    check(session::PostgresStore::new(pool.clone())).await;
    check_devices(session::PostgresStore::new(pool)).await;
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn mysql() {
    let Ok(url) = std::env::var("SESSION_MYSQL_URL") else { return eprintln!("SESSION_MYSQL_URL not set; skipping") };
    let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
    sqlx::raw_sql("drop table if exists sessions").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/mysql/create_sessions.up.sql")).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/mysql/add_session_devices.up.sql")).execute(&pool).await.unwrap();
    check(session::MysqlStore::new(pool.clone())).await;
    check_devices(session::MysqlStore::new(pool)).await;
}

#[cfg(feature = "redis")]
#[tokio::test]
async fn redis() {
    let Ok(url) = std::env::var("SESSION_REDIS_URL") else { return eprintln!("SESSION_REDIS_URL not set; skipping") };
    let conn = redis::Client::open(url).unwrap().get_connection_manager().await.unwrap();
    let store = session::RedisStore::new(conn).prefix("test:session:");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let live = Record { id: id(1), data: b"live".to_vec(), expires_at: now + 3600, user_id: None, device: None };
    store.save(&live).await.unwrap();
    store.save(&Record { data: b"updated".to_vec(), ..live.clone() }).await.unwrap();
    let loaded = store.load(&id(1)).await.unwrap().unwrap();
    assert_eq!((loaded.data.as_slice(), loaded.expires_at), (&b"updated"[..], now + 3600), "upsert keeps the expiry");

    store.save(&Record { id: id(2), data: b"dead".to_vec(), expires_at: now - 10, user_id: None, device: None }).await.unwrap();
    assert!(store.load(&id(2)).await.unwrap().is_none(), "an expired record is not kept");

    store.delete(&id(1)).await.unwrap();
    assert!(store.load(&id(1)).await.unwrap().is_none());
}
