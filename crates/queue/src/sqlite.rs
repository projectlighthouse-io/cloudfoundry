use std::time::Duration;

use sqlx::{SqliteConnection, SqlitePool};

use crate::{Claimed, Error, Job, JobId, NewJob, Queue, Store, new_job, placeholders};

/// Jobs in SQLite (`art publish queue --driver sqlite`). SQLite has one
/// writer at a time, so a single `UPDATE ... RETURNING` claims atomically.
#[derive(Clone)]
pub struct SqliteStore(SqlitePool);

impl SqliteStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self(pool)
    }
}

const NOW: &str = "cast(strftime('%s', 'now') as integer)";

async fn insert(conn: &mut SqliteConnection, job: &NewJob) -> Result<JobId, sqlx::Error> {
    let sql = format!(
        "insert into jobs (queue, name, payload, max_attempts, available_at, created_at)
         values (?, ?, ?, ?, {NOW} + ?, {NOW}) returning id"
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

impl Store for SqliteStore {
    async fn push(&self, job: NewJob) -> Result<JobId, Error> {
        let mut conn = self.0.acquire().await?;
        Ok(insert(&mut conn, &job).await?)
    }

    async fn claim(&self, queues: &[String], limit: u32, visibility: Duration) -> Result<Vec<Claimed>, Error> {
        if queues.is_empty() {
            return Ok(vec![]);
        }
        let sql = format!(
            "update jobs set reserved_until = {NOW} + ?, attempts = attempts + 1
             where id in (
                 select id from jobs
                 where queue in ({}) and available_at <= {NOW}
                   and (reserved_until is null or reserved_until < {NOW})
                 order by id limit ?
             )
             returning id, queue, name, payload, attempts, max_attempts",
            placeholders(queues.len())
        );
        let mut q = sqlx::query_as::<_, (i64, String, String, String, i32, i32)>(&sql).bind(visibility.as_secs() as i64);
        for queue in queues {
            q = q.bind(queue);
        }
        let rows = q.bind(limit as i64).fetch_all(&self.0).await?;
        Ok(crate::claimed(rows))
    }

    async fn complete(&self, id: JobId) -> Result<(), Error> {
        sqlx::query("delete from jobs where id = ?").bind(id.0).execute(&self.0).await?;
        Ok(())
    }

    async fn fail(&self, id: JobId, error: &str, retry_in: Option<Duration>) -> Result<(), Error> {
        if let Some(delay) = retry_in {
            let sql = format!("update jobs set reserved_until = null, available_at = {NOW} + ?, last_error = ? where id = ?");
            sqlx::query(&sql).bind(delay.as_secs() as i64).bind(error).bind(id.0).execute(&self.0).await?;
            return Ok(());
        }
        let mut tx = self.0.begin().await?;
        let sql = format!(
            "insert into failed_jobs (id, queue, name, payload, attempts, max_attempts, error, failed_at)
             select id, queue, name, payload, attempts, max_attempts, ?, {NOW} from jobs where id = ?"
        );
        sqlx::query(&sql).bind(error).bind(id.0).execute(&mut *tx).await?;
        sqlx::query("delete from jobs where id = ?").bind(id.0).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn retry_failed(&self, id: JobId) -> Result<Option<JobId>, Error> {
        let mut tx = self.0.begin().await?;
        let sql = format!(
            "insert into jobs (queue, name, payload, max_attempts, available_at, created_at)
             select queue, name, payload, max_attempts, {NOW}, {NOW} from failed_jobs where id = ?
             returning id"
        );
        let row: Option<(i64,)> = sqlx::query_as(&sql).bind(id.0).fetch_optional(&mut *tx).await?;
        sqlx::query("delete from failed_jobs where id = ?").bind(id.0).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(row.map(|(id,)| JobId(id)))
    }
}

impl Queue<SqliteStore> {
    /// Enqueue inside the caller's transaction.
    pub async fn push_in_tx<J: Job>(&self, conn: &mut SqliteConnection, job: J) -> Result<JobId, Error> {
        Ok(insert(conn, &new_job(&job, Duration::ZERO)?).await?)
    }
}
