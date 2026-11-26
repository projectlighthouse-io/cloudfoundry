use std::time::Duration;

use sqlx::{PgConnection, PgPool};

use crate::{Claimed, Error, Job, JobId, NewJob, Queue, Store, new_job};

/// Jobs in the `jobs` and `failed_jobs` tables (`art publish queue`).
/// Claims with `FOR UPDATE SKIP LOCKED`, so workers never block each other.
#[derive(Clone)]
pub struct PostgresStore(PgPool);

impl PostgresStore {
    pub fn new(pool: PgPool) -> Self {
        Self(pool)
    }
}

const NOW: &str = "extract(epoch from now())::bigint";

async fn insert(conn: &mut PgConnection, job: &NewJob) -> Result<JobId, sqlx::Error> {
    let sql = format!(
        "insert into jobs (queue, name, payload, max_attempts, available_at, created_at)
         values ($1, $2, $3, $4, {NOW} + $5, {NOW}) returning id"
    );
    let (id,): (i64,) = sqlx::query_as(&sql)
        .bind(&job.queue)
        .bind(&job.name)
        .bind(&job.payload)
        .bind(job.max_attempts as i32)
        .bind(job.delay.as_secs() as i64)
        .fetch_one(conn)
        .await?;
    Ok(JobId(id))
}

impl Store for PostgresStore {
    async fn push(&self, job: NewJob) -> Result<JobId, Error> {
        let mut conn = self.0.acquire().await?;
        Ok(insert(&mut conn, &job).await?)
    }

    async fn claim(&self, queues: &[String], limit: u32, visibility: Duration) -> Result<Vec<Claimed>, Error> {
        let sql = format!(
            "update jobs set reserved_until = {NOW} + $3, attempts = attempts + 1
             where id in (
                 select id from jobs
                 where queue = any($1) and available_at <= {NOW}
                   and (reserved_until is null or reserved_until < {NOW})
                 order by id
                 for update skip locked
                 limit $2
             )
             returning id, queue, name, payload, attempts, max_attempts"
        );
        let rows: Vec<(i64, String, String, String, i32, i32)> = sqlx::query_as(&sql)
            .bind(queues)
            .bind(limit as i64)
            .bind(visibility.as_secs() as i64)
            .fetch_all(&self.0)
            .await?;
        Ok(crate::claimed(rows))
    }

    async fn complete(&self, id: JobId) -> Result<(), Error> {
        sqlx::query("delete from jobs where id = $1").bind(id.0).execute(&self.0).await?;
        Ok(())
    }

    async fn fail(&self, id: JobId, error: &str, retry_in: Option<Duration>) -> Result<(), Error> {
        match retry_in {
            Some(delay) => {
                let sql = format!("update jobs set reserved_until = null, available_at = {NOW} + $2, last_error = $3 where id = $1");
                sqlx::query(&sql).bind(id.0).bind(delay.as_secs() as i64).bind(error).execute(&self.0).await?;
            }
            None => {
                let sql = format!(
                    "with moved as (delete from jobs where id = $1 returning *)
                     insert into failed_jobs (id, queue, name, payload, attempts, max_attempts, error, failed_at)
                     select id, queue, name, payload, attempts, max_attempts, $2, {NOW} from moved"
                );
                sqlx::query(&sql).bind(id.0).bind(error).execute(&self.0).await?;
            }
        }
        Ok(())
    }

    async fn extend(&self, id: JobId, visibility: Duration) -> Result<(), Error> {
        let sql = format!("update jobs set reserved_until = {NOW} + $2 where id = $1");
        sqlx::query(&sql).bind(visibility.as_secs() as i64).bind(id.0).execute(&self.0).await?;
        Ok(())
    }

    async fn retry_failed(&self, id: JobId) -> Result<Option<JobId>, Error> {
        let sql = format!(
            "with moved as (delete from failed_jobs where id = $1 returning *)
             insert into jobs (queue, name, payload, max_attempts, available_at, created_at)
             select queue, name, payload, max_attempts, {NOW}, {NOW} from moved
             returning id"
        );
        let row: Option<(i64,)> = sqlx::query_as(&sql).bind(id.0).fetch_optional(&self.0).await?;
        Ok(row.map(|(id,)| JobId(id)))
    }
}

impl Queue<PostgresStore> {
    /// Enqueue inside the caller's transaction: if it rolls back, the job was
    /// never queued.
    pub async fn push_in_tx<J: Job>(&self, conn: &mut PgConnection, job: J) -> Result<JobId, Error> {
        Ok(insert(conn, &new_job(&job, Duration::ZERO)?).await?)
    }
}
