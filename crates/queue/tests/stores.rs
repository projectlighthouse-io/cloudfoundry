//! The same checks against every SQL store. SQLite runs always (in memory);
//! Postgres and MySQL run when QUEUE_PG_URL / QUEUE_MYSQL_URL are set.

#![cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]

use std::time::Duration;

use queue::{Job, JobError, JobId, Queue, Store};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Ping(u32);
impl Job for Ping {
    const NAME: &'static str = "ping";
    type Context = ();
    async fn run(self, _: &()) -> Result<(), JobError> {
        Ok(())
    }
}

async fn check<St: Store>(q: &Queue<St>) {
    let queues = ["default".to_owned()];
    let hold = Duration::from_secs(60);
    for i in 0..4 {
        q.push(Ping(i)).await.unwrap();
    }
    q.push_in(Duration::from_secs(3600), Ping(99)).await.unwrap();

    // Two concurrent claims never get the same job, and the delayed one waits.
    let (a, b) = tokio::join!(q.store().claim(&queues, 3, hold), q.store().claim(&queues, 3, hold));
    let mut ids: Vec<JobId> = a.unwrap().iter().chain(b.unwrap().iter()).map(|j| j.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 4, "no overlap, nothing delayed");
    assert!(q.store().claim(&["other".to_owned()], 10, hold).await.unwrap().is_empty(), "queues are separate");

    // Held jobs are not claimable; a released one is, with attempts counted.
    assert!(q.store().claim(&queues, 10, hold).await.unwrap().is_empty());
    q.store().fail(ids[0], "try again", Some(Duration::ZERO)).await.unwrap();
    let again = q.store().claim(&queues, 10, hold).await.unwrap();
    assert_eq!((again.len(), again[0].attempts, again[0].payload.as_str()), (1, 2, "0"));

    // Complete one, fail one for good, retry it exactly once.
    q.store().extend(ids[3], hold).await.unwrap();
    q.store().complete(ids[1]).await.unwrap();
    q.store().fail(ids[2], "dead", None).await.unwrap();
    let requeued = q.store().retry_failed(ids[2]).await.unwrap().expect("was failed");
    assert!(q.store().retry_failed(ids[2]).await.unwrap().is_none(), "only once");
    let back = q.store().claim(&queues, 10, hold).await.unwrap();
    assert!(back.iter().any(|j| j.id == requeued && j.attempts == 1 && j.name == "ping"));
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/sqlite/create_jobs.up.sql")).execute(&pool).await.unwrap();
    let q = Queue::new(queue::SqliteStore::new(pool.clone()));
    check(&q).await;

    let mut tx = pool.begin().await.unwrap();
    q.push_in_tx(&mut tx, Ping(7)).await.unwrap();
    tx.rollback().await.unwrap();
    let (n,): (i64,) = sqlx::query_as("select count(*) from jobs where payload = '7'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0, "rolled back means never queued");
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres() {
    let Ok(url) = std::env::var("QUEUE_PG_URL") else { return eprintln!("QUEUE_PG_URL not set; skipping") };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql("drop table if exists failed_jobs; drop table if exists jobs").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/create_jobs.up.sql")).execute(&pool).await.unwrap();
    let q = Queue::new(queue::PostgresStore::new(pool.clone()));
    check(&q).await;

    let mut tx = pool.begin().await.unwrap();
    q.push_in_tx(&mut tx, Ping(7)).await.unwrap();
    tx.rollback().await.unwrap();
    let (n,): (i64,) = sqlx::query_as("select count(*) from jobs where payload = '7'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0, "rolled back means never queued");
}

#[cfg(feature = "mysql")]
#[tokio::test(flavor = "multi_thread")]
async fn mysql() {
    let Ok(url) = std::env::var("QUEUE_MYSQL_URL") else { return eprintln!("QUEUE_MYSQL_URL not set; skipping") };
    let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
    sqlx::raw_sql("drop table if exists failed_jobs; drop table if exists jobs").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/mysql/create_jobs.up.sql")).execute(&pool).await.unwrap();
    let q = Queue::new(queue::MysqlStore::new(pool.clone()));
    check(&q).await;

    let mut tx = pool.begin().await.unwrap();
    q.push_in_tx(&mut tx, Ping(7)).await.unwrap();
    tx.rollback().await.unwrap();
    let (n,): (i64,) = sqlx::query_as("select count(*) from jobs where payload = '7'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0, "rolled back means never queued");
}
