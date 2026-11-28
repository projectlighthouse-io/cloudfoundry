#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
//! `Encrypted<T>` as a column and `Rotate` on every driver. SQLite always;
//! Postgres and MySQL when CRYPT_PG_URL / CRYPT_MYSQL_URL are set.
#![cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]

use crypt::{
    Encrypted, Encrypter, Keys,
    rotate::{Column, Rotate, Rotated},
};

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Secret {
    code: String,
}

fn keys(list: &str) -> Encrypter {
    Encrypter::new(Keys::parse(list).unwrap())
}

macro_rules! check {
    ($pool:expr, $create:literal, $ins:literal, $sel:literal) => {{
        let pool = $pool;
        sqlx::raw_sql("drop table if exists vault").execute(&pool).await.unwrap();
        sqlx::raw_sql($create).execute(&pool).await.unwrap();
        let (old, new) = (Keys::generate(), Keys::generate());
        let before = keys(&old);
        // 1200 rows: more than two pages. Each sealed for its own id.
        for id in 1..=1200i64 {
            let value = Encrypted::seal(&before, format!("vault:{id}").as_bytes(), &Secret { code: format!("c{id}") }).unwrap();
            sqlx::query($ins).bind(id).bind(value).execute(&pool).await.unwrap();
        }
        sqlx::query($ins).bind(5000i64).bind(vec![1u8, 2, 3]).execute(&pool).await.unwrap();

        let both = keys(&format!("{new},{old}"));
        let column = Column { table: "vault", column: "secret", id: "id", purpose: "vault:{id}" };
        let dry = pool.rotate(&both, &column, true).await.unwrap();
        assert_eq!(dry, Rotated { rotated: 1200, current: 0, failed: 1 }, "dry run counts");
        let done = pool.rotate(&both, &column, false).await.unwrap();
        assert_eq!(done, Rotated { rotated: 1200, current: 0, failed: 1 });
        assert_eq!(pool.rotate(&both, &column, false).await.unwrap(), Rotated { rotated: 0, current: 1200, failed: 1 }, "idempotent");

        // The old key can go now.
        let only_new = keys(&new);
        let row: (Encrypted<Secret>,) = sqlx::query_as($sel).bind(777i64).fetch_one(&pool).await.unwrap();
        assert_eq!(row.0.open(&only_new, b"vault:777").unwrap(), Secret { code: "c777".into() });
        assert!(!row.0.needs_rotation(&only_new));
        assert_eq!(format!("{:?}", row.0), "…");

        let wrong = Column { purpose: "other:{id}", ..column.clone() };
        assert!(keys(&format!("{},{new}", Keys::generate())).needs_rotation(row.0.as_bytes()));
        assert_eq!(pool.rotate(&keys(&format!("{},{new}", Keys::generate())), &wrong, true).await.unwrap().failed, 1201, "wrong purpose: nothing opens");
        assert!(pool.rotate(&both, &Column { table: "vault; drop table vault", ..column.clone() }, true).await.is_err());
    }};
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    check!(pool, "create table vault (id integer primary key, secret blob)", "insert into vault (id, secret) values (?, ?)", "select secret from vault where id = ?");
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres() {
    let Ok(url) = std::env::var("CRYPT_PG_URL") else { return eprintln!("CRYPT_PG_URL not set; skipping") };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    check!(pool, "create table vault (id bigint primary key, secret bytea)", "insert into vault (id, secret) values ($1, $2)", "select secret from vault where id = $1");
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn mysql() {
    let Ok(url) = std::env::var("CRYPT_MYSQL_URL") else { return eprintln!("CRYPT_MYSQL_URL not set; skipping") };
    let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
    check!(pool, "create table vault (id bigint primary key, secret varbinary(1024))", "insert into vault (id, secret) values (?, ?)", "select secret from vault where id = ?");
}
