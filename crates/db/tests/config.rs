#![allow(clippy::unwrap_used)]
use std::time::Duration;

use db::PoolConfig;

#[test]
fn check_rejects_impossible_pools() {
    let d = PoolConfig::default;
    assert!(d().check().is_ok());
    assert!(PoolConfig { max_connections: 0, ..d() }.check().is_err());
    assert!(PoolConfig { min_connections: 11, ..d() }.check().is_err());
    assert!(PoolConfig { acquire_timeout: Duration::ZERO, ..d() }.check().is_err());
    assert!(PoolConfig { idle_timeout: Some(Duration::ZERO), ..d() }.check().is_err());
    assert!(PoolConfig { idle_timeout: None, max_lifetime: None, ..d() }.check().is_ok());
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn connect_with_applies_and_checks() {
    db::handle!(Mem: Sqlite);
    let cfg = PoolConfig { max_connections: 1, ..PoolConfig::default() };
    let db = db::Db::<Mem>::connect_with("sqlite::memory:", &cfg).await.unwrap();
    assert_eq!(db.pool().options().get_max_connections(), 1);

    let bad = PoolConfig { max_connections: 0, ..PoolConfig::default() };
    let err = db::Db::<Mem>::connect_with("sqlite::memory:", &bad).await.unwrap_err();
    assert!(matches!(err, sqlx::Error::Configuration(_)), "{err}");
}

#[cfg(feature = "configured")]
mod serde_tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct App {
        #[serde(default)]
        db: PoolConfig,
    }

    #[test]
    fn partial_keeps_defaults() {
        let app: App = toml::from_str("[db]\nmax_connections = 20\nacquire_timeout = \"5s\"\n").unwrap();
        let want = PoolConfig { max_connections: 20, acquire_timeout: Duration::from_secs(5), ..PoolConfig::default() };
        assert_eq!(app.db, want);
        assert_eq!(toml::from_str::<App>("").unwrap().db, PoolConfig::default());
    }

    #[test]
    fn never_turns_a_timeout_off() {
        let app: App = toml::from_str("[db]\nidle_timeout = \"never\"\n").unwrap();
        assert_eq!(app.db.idle_timeout, None);
        assert_eq!(app.db.max_lifetime, PoolConfig::default().max_lifetime);
    }

    #[test]
    fn round_trips() {
        let cfg = PoolConfig { idle_timeout: None, ..PoolConfig::default() };
        let back: PoolConfig = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn unknown_key_is_rejected() {
        assert!(toml::from_str::<App>("[db]\nmax_conections = 2\n").is_err());
    }
}
