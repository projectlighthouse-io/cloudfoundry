#![cfg(all(feature = "uuid", feature = "sqlite"))]
#![allow(clippy::unwrap_used)] // test

use db::Uuid;

struct Machine;

#[test]
fn parses_displays_and_rejects_numbers() {
    let id = Uuid::<Machine>::new();
    assert_eq!(id.as_uuid().get_version_num(), 7);
    assert_eq!(id.to_string().parse::<Uuid<Machine>>().unwrap(), id);
    assert!("42".parse::<Uuid<Machine>>().is_err());
    assert_eq!(serde_json::to_string(&id).unwrap(), format!("\"{id}\""));
    assert_eq!(serde_json::from_str::<Uuid<Machine>>(&format!("\"{id}\"")).unwrap(), id);
    assert!(serde_json::from_str::<Uuid<Machine>>("42").is_err(), "a number is not a uuid");
}

#[tokio::test]
async fn round_trips_through_sqlite() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("create table machines (id integer primary key, uuid blob not null unique)").execute(&pool).await.unwrap();
    let id = Uuid::<Machine>::new();
    sqlx::query("insert into machines (uuid) values (?)").bind(id).execute(&pool).await.unwrap();
    let (back,): (Uuid<Machine>,) = sqlx::query_as("select uuid from machines where uuid = ?").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(back, id);
}

#[cfg(feature = "ts")]
#[test]
fn typescript_is_a_branded_string_in_its_own_file() {
    use ts_rs::TS;

    #[derive(TS)]
    #[allow(dead_code)]
    struct Item {
        uuid: Uuid<Machine>,
        parent: Option<Uuid<Machine>>,
    }

    let cfg = ts_rs::Config::default();
    assert_eq!(Item::inline(&cfg), r#"{ uuid: Uuid<"Machine">, parent: Uuid<"Machine"> | null, }"#);
    let deps = Item::dependencies(&cfg);
    assert!(deps.iter().any(|d| d.ts_name == "Uuid" && d.output_path == std::path::Path::new("Uuid.ts")), "imported from Uuid.ts");
    assert_eq!(Uuid::<Machine>::decl(&cfg), "type Uuid<T extends string> = string & { readonly __uuid: T };");
    // `art new` ships this file; it must match what ts-rs writes.
    let dir = std::env::temp_dir().join(format!("db-ts-{}", std::process::id()));
    Uuid::<Machine>::export(&ts_rs::Config::default().with_out_dir(&dir)).unwrap();
    let written = std::fs::read_to_string(dir.join("Uuid.ts")).unwrap();
    // Keep in step with `UUID_TS` in art/src/new.rs.
    assert_eq!(written, include_str!("uuid.ts"));
    std::fs::remove_dir_all(&dir).unwrap();
}
