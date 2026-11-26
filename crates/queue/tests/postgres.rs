#![cfg(feature = "postgres")]
//! Needs a database: `QUEUE_DATABASE_URL=postgres://... cargo test -p queue --features postgres`.
//! Skipped when the variable is unset.

use std::time::Duration;

use queue::{Job, JobError, JobId, PostgresStore, Queue, Store};
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

#[tokio::test(flavor = "multi_thread")]
async fn postgres_store() {
    let Ok(url) = std::env::var("QUEUE_DATABASE_URL") else {
        eprintln!("QUEUE_DATABASE_URL not set; skipping");
        return;
    };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/create_jobs.down.sql")).execute(&pool).await.ok();
    sqlx::raw_sql(include_str!("../stubs/migrations/create_jobs.up.sql")).execute(&pool).await.unwrap();
    let q = Queue::new(PostgresStore::new(pool.clone()));
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
    assert_eq!(ids.len(), 4, "SKIP LOCKED: no overlap, nothing delayed");

    // Held jobs are not claimable; a released one is, with attempts counted.
    assert!(q.store().claim(&queues, 10, hold).await.unwrap().is_empty());
    q.store().fail(ids[0], "try again", Some(Duration::ZERO)).await.unwrap();
    let again = q.store().claim(&queues, 10, hold).await.unwrap();
    assert_eq!((again.len(), again[0].attempts), (1, 2));

    // Complete one, fail one for good, then retry it.
    q.store().complete(ids[1]).await.unwrap();
    q.store().fail(ids[2], "dead", None).await.unwrap();
    let (n,): (i64,) = sqlx::query_as("select count(*) from failed_jobs where error = 'dead'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    let requeued = q.store().retry_failed(ids[2]).await.unwrap().expect("was failed");
    assert!(q.store().retry_failed(ids[2]).await.unwrap().is_none(), "only once");
    let back = q.store().claim(&queues, 10, hold).await.unwrap();
    assert!(back.iter().any(|j| j.id == requeued && j.attempts == 1));

    // push_in_tx: rolled back means never queued.
    let before: (i64,) = sqlx::query_as("select count(*) from jobs").fetch_one(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    q.push_in_tx(&mut tx, Ping(7)).await.unwrap();
    tx.rollback().await.unwrap();
    let after: (i64,) = sqlx::query_as("select count(*) from jobs").fetch_one(&pool).await.unwrap();
    assert_eq!(before, after);
}
