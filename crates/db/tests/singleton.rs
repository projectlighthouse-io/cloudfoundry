#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
#![cfg(all(feature = "singleton", feature = "sqlite"))]

use db::{Db, SingletonError};

db::handle!(Single: Sqlite);

#[tokio::test]
async fn singleton_reports_errors_instead_of_panicking() {
    assert_eq!(Db::<Single>::singleton().err(), Some(SingletonError::NotSet("Single")));

    let first = Db::<Single>::connect("sqlite::memory:").await.unwrap();
    assert_eq!(Db::<Single>::set_singleton(first), Ok(()));
    assert!(Db::<Single>::singleton().is_ok());

    let second = Db::<Single>::connect("sqlite::memory:").await.unwrap();
    let err = Db::<Single>::set_singleton(second).unwrap_err();
    assert_eq!(err, SingletonError::AlreadySet("Single"));
    assert_eq!(err.to_string(), "db singleton for `Single` is already set");
}
