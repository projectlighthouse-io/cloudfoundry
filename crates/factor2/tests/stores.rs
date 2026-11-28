#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
//! The same checks against every store. Memory and SQLite run always;
//! Postgres and MySQL run when FACTOR2_PG_URL / FACTOR2_MYSQL_URL are set.

use factor2::{MemoryStore, NewDevice, Store, Uuid};

async fn check<S: Store>(s: S) {
    // Secrets: unconfirmed, confirmed once, steps only move forward.
    assert_eq!(s.secret(1).await.unwrap(), None);
    s.put_secret(1, b"sealed-1", 100).await.unwrap();
    s.put_secret(1, b"sealed-2", 101).await.unwrap();
    let secret = s.secret(1).await.unwrap().unwrap();
    assert_eq!((secret.sealed.as_slice(), secret.confirmed_at, secret.last_step), (&b"sealed-2"[..], None, None));
    assert!(!s.use_step(1, 5).await.unwrap(), "unconfirmed secrets take no codes");
    assert!(s.confirm_secret(1, 200, 10).await.unwrap());
    assert!(!s.confirm_secret(1, 201, 11).await.unwrap(), "confirmed once");
    assert_eq!(s.secret(1).await.unwrap().unwrap().last_step, Some(10));
    assert!(!s.use_step(1, 10).await.unwrap(), "same step: replay");
    assert!(!s.use_step(1, 9).await.unwrap(), "older step");
    assert!(s.use_step(1, 11).await.unwrap());
    assert!(!s.use_step(2, 11).await.unwrap(), "no secret");

    // Backup codes: single use, replaced as a set.
    s.replace_backup_codes(1, &[vec![1; 32], vec![2; 32]]).await.unwrap();
    assert_eq!(s.unused_backup_codes(1).await.unwrap(), 2);
    assert!(s.use_backup_code(1, &[1; 32], 300).await.unwrap());
    assert!(!s.use_backup_code(1, &[1; 32], 301).await.unwrap(), "used");
    assert!(!s.use_backup_code(2, &[2; 32], 301).await.unwrap(), "another user's");
    assert_eq!(s.unused_backup_codes(1).await.unwrap(), 1);
    s.replace_backup_codes(1, &[vec![3; 32]]).await.unwrap();
    assert!(!s.use_backup_code(1, &[2; 32], 302).await.unwrap(), "replaced");
    assert_eq!(s.unused_backup_codes(1).await.unwrap(), 1);

    // Trusted devices.
    let (a, b) = (Uuid::new(), Uuid::new());
    for (uuid, token) in [(a, 7u8), (b, 8)] {
        s.add_device(NewDevice { uuid, user_id: 1, token_hash: vec![token; 32], user_agent: Some("Firefox".into()), ip: Some("::1".into()), expires_at: 999, created_at: 400 })
            .await
            .unwrap();
    }
    let found = s.device_by_token(&[7; 32]).await.unwrap().unwrap();
    assert_eq!((found.uuid, found.user_id, found.user_agent.as_deref(), found.ip.as_deref(), found.expires_at), (a, 1, Some("Firefox"), Some("::1"), 999));
    assert_eq!(found.last_used_at, None);
    s.touch_device(found.id, 500).await.unwrap();
    assert_eq!(s.device_by_token(&[7; 32]).await.unwrap().unwrap().last_used_at, Some(500));
    assert_eq!(s.devices(1).await.unwrap().iter().map(|d| d.uuid).collect::<Vec<_>>(), [b, a], "newest first");
    assert!(!s.delete_device(2, a).await.unwrap(), "not theirs");
    assert!(s.delete_device(1, a).await.unwrap());
    assert!(s.device_by_token(&[7; 32]).await.unwrap().is_none());

    s.delete_all(1).await.unwrap();
    assert_eq!(s.secret(1).await.unwrap(), None);
    assert_eq!(s.unused_backup_codes(1).await.unwrap(), 0);
    assert!(s.devices(1).await.unwrap().is_empty());
}

#[tokio::test]
async fn memory() {
    check(MemoryStore::new()).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql("create table users (id integer primary key); insert into users (id) values (1), (2)").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/sqlite/create_two_factor.up.sql")).execute(&pool).await.unwrap();
    check(factor2::SqliteStore::new(pool)).await;
}

#[cfg(any(feature = "postgres", feature = "mysql"))]
const RESET: &str = "drop table if exists trusted_devices; drop table if exists backup_codes; drop table if exists totp_secrets; drop table if exists users;
create table users (id bigint primary key); insert into users (id) values (1), (2)";

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres() {
    let Ok(url) = std::env::var("FACTOR2_PG_URL") else { return eprintln!("FACTOR2_PG_URL not set; skipping") };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql(RESET).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/create_two_factor.up.sql")).execute(&pool).await.unwrap();
    check(factor2::PostgresStore::new(pool)).await;
}

#[cfg(feature = "mysql")]
#[tokio::test(flavor = "multi_thread")]
async fn mysql() {
    let Ok(url) = std::env::var("FACTOR2_MYSQL_URL") else { return eprintln!("FACTOR2_MYSQL_URL not set; skipping") };
    let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
    sqlx::raw_sql(RESET).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/mysql/create_two_factor.up.sql")).execute(&pool).await.unwrap();
    check(factor2::MysqlStore::new(pool)).await;
}
