#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
//! The same checks against every store. Memory and SQLite run always;
//! Postgres and MySQL run when PASSKEYS_PG_URL / PASSKEYS_MYSQL_URL are set.

use passkeys::{Credential, MemoryStore, NewPasskey, Passkeys, Store, Uuid};
use session::Session;
use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};
use webauthn_rs::prelude::Url;

/// A real credential, from a software authenticator.
async fn credential() -> Credential {
    let pk = Passkeys::new(MemoryStore::new(), "https://acme.test", "Acme").unwrap();
    let session = Session::new();
    let options = pk.start_registration(&session, 1, "ada", "Ada").await.unwrap();
    let answer = SoftPasskey::new(true).do_registration(Url::parse("https://acme.test").unwrap(), options).unwrap();
    pk.finish_registration(&session, 1, "k", &answer).await.unwrap().credential
}

fn new(user_id: i64, credential_id: u8, credential: &Credential, created_at: i64) -> NewPasskey {
    NewPasskey {
        public_id: Uuid::new(),
        user_id,
        name: format!("key {credential_id}"),
        credential_id: vec![credential_id; 16],
        user_handle: uuid::Uuid::from_bytes([user_id as u8; 16]),
        credential: credential.clone(),
        created_at,
    }
}

async fn check<S: Store>(s: S) {
    let cred = credential().await;
    let (a, b, c) = (new(1, 1, &cred, 100), new(1, 2, &cred, 101), new(2, 3, &cred, 102));
    for p in [&a, &b, &c] {
        s.add(p.clone()).await.unwrap();
    }
    assert!(s.add(new(2, 1, &cred, 103)).await.is_err(), "credential ids are unique");

    let found = s.by_credential_id(&[1; 16]).await.unwrap().unwrap();
    assert_eq!((found.public_id, found.user_id, found.name.as_str(), found.user_handle, found.last_used_at, found.created_at), (a.public_id, 1, "key 1", a.user_handle, None, 100));
    assert_eq!(found.credential.cred_id(), cred.cred_id(), "credential round-trips");
    assert!(s.by_credential_id(&[9; 16]).await.unwrap().is_none());
    assert_eq!(s.passkeys(1).await.unwrap().iter().map(|p| p.public_id).collect::<Vec<_>>(), [b.public_id, a.public_id], "newest first");

    s.used(found.id, &cred, 500).await.unwrap();
    assert_eq!(s.by_credential_id(&[1; 16]).await.unwrap().unwrap().last_used_at, Some(500));

    assert!(!s.rename(2, a.public_id, "theirs").await.unwrap(), "not theirs");
    assert!(s.rename(1, a.public_id, "Laptop").await.unwrap());
    assert!(s.rename(1, a.public_id, "Laptop").await.unwrap(), "same name still found");
    assert_eq!(s.by_credential_id(&[1; 16]).await.unwrap().unwrap().name, "Laptop");

    assert!(!s.delete(2, a.public_id).await.unwrap(), "not theirs");
    assert!(s.delete(1, a.public_id).await.unwrap());
    assert!(!s.delete(1, a.public_id).await.unwrap(), "gone");
    s.delete_all(1).await.unwrap();
    assert!(s.passkeys(1).await.unwrap().is_empty());
    assert_eq!(s.passkeys(2).await.unwrap().len(), 1, "other users keep theirs");
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
    sqlx::raw_sql(include_str!("../stubs/migrations/sqlite/create_passkeys.up.sql")).execute(&pool).await.unwrap();
    check(passkeys::SqliteStore::new(pool)).await;
}

#[cfg(any(feature = "postgres", feature = "mysql"))]
const RESET: &str = "drop table if exists passkeys; drop table if exists users;
create table users (id bigint primary key); insert into users (id) values (1), (2)";

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres() {
    let Ok(url) = std::env::var("PASSKEYS_PG_URL") else { return eprintln!("PASSKEYS_PG_URL not set; skipping") };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql(RESET).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/create_passkeys.up.sql")).execute(&pool).await.unwrap();
    check(passkeys::PostgresStore::new(pool)).await;
}

#[cfg(feature = "mysql")]
#[tokio::test(flavor = "multi_thread")]
async fn mysql() {
    let Ok(url) = std::env::var("PASSKEYS_MYSQL_URL") else { return eprintln!("PASSKEYS_MYSQL_URL not set; skipping") };
    let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
    sqlx::raw_sql(RESET).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/mysql/create_passkeys.up.sql")).execute(&pool).await.unwrap();
    check(passkeys::MysqlStore::new(pool)).await;
}
