#![cfg(feature = "sqlite")]

use db::Db;

db::handle!(Mem: Sqlite);

#[tokio::test]
async fn tx_commits_on_ok_and_rolls_back_on_err() {
    // One connection, so the in-memory database is shared across queries.
    let pool = sqlx::pool::PoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    let db: Db<Mem> = Db::from_pool(pool);
    sqlx::query("create table t (v integer)").execute(db.pool()).await.unwrap();

    db.tx(async |tx| sqlx::query("insert into t values (1)").execute(&mut **tx).await.map(drop))
        .await
        .unwrap();

    let err: Result<(), sqlx::Error> = db
        .tx(async |tx| {
            sqlx::query("insert into t values (2)").execute(&mut **tx).await?;
            Err(sqlx::Error::RowNotFound)
        })
        .await;
    assert!(err.is_err());

    let (n,): (i64,) = sqlx::query_as("select count(*) from t").fetch_one(db.pool()).await.unwrap();
    assert_eq!(n, 1);
}
