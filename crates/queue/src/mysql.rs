use std::time::Duration;

use sqlx::{MySqlConnection, MySqlPool};

use crate::{Claimed, Error, Job, JobId, NewJob, Queue, Store, new_job, placeholders};

/// Jobs in MySQL 8+ (`art publish queue --driver mysql`). Claims with
/// `FOR UPDATE SKIP LOCKED` inside a transaction (MySQL has no `RETURNING`).
#[derive(Clone)]
pub struct MysqlStore(MySqlPool);

impl MysqlStore {
    pub fn new(pool: MySqlPool) -> Self {
        Self(pool)
    }
}

const NOW: &str = "unix_timestamp()";

async fn insert(conn: &mut MySqlConnection, job: &NewJob) -> Result<JobId, sqlx::Error> {
    let sql = format!(
        "insert into jobs (queue, name, payload, max_attempts, available_at, created_at)
         values (?, ?, ?, ?, {NOW} + ?, {NOW})"
    );
    let res = sqlx::query(&sql)
        .bind(&job.queue)
        .bind(&job.name)
        .bind(&job.payload)
        .bind(job.max_attempts as i32)
        .bind(job.delay.as_secs() as i64)
        .execute(conn)
        .await?;
    Ok(JobId(res.last_insert_id() as i64))
}

impl Store for MysqlStore {
    async fn push(&self, job: NewJob) -> Result<JobId, Error> {
        let mut conn = self.0.acquire().await?;
        Ok(insert(&mut conn, &job).await?)
    }

    async fn claim(&self, queues: &[String], limit: u32, visibility: Duration) -> Result<Vec<Claimed>, Error> {
        if queues.is_empty() {
            return Ok(vec![]);
        }
        let mut tx = self.0.begin().await?;
        let select = format!(
            "select id from jobs
             where queue in ({}) and available_at <= {NOW}
               and (reserved_until is null or reserved_until < {NOW})
             order by id limit ?
             for update skip locked",
            placeholders(queues.len())
        );
        let mut q = sqlx::query_as::<_, (i64,)>(&select);
        for queue in queues {
            q = q.bind(queue);
        }
        let ids: Vec<i64> = q.bind(limit as i64).fetch_all(&mut *tx).await?.into_iter().map(|(id,)| id).collect();
        if ids.is_empty() {
            tx.commit().await?;
            return Ok(vec![]);
        }

        let list = placeholders(ids.len());
        let update = format!("update jobs set reserved_until = {NOW} + ?, attempts = attempts + 1 where id in ({list})");
        let mut u = sqlx::query(&update).bind(visibility.as_secs() as i64);
        for id in &ids {
            u = u.bind(id);
        }
        u.execute(&mut *tx).await?;

        let read = format!("select id, queue, name, payload, attempts, max_attempts from jobs where id in ({list})");
        let mut r = sqlx::query_as::<_, (i64, String, String, String, i32, i32)>(&read);
        for id in &ids {
            r = r.bind(id);
        }
        let rows = r.fetch_all(&mut *tx).await?;
        tx.commit().await?;
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

    async fn extend(&self, id: JobId, visibility: Duration) -> Result<(), Error> {
        let sql = format!("update jobs set reserved_until = {NOW} + ? where id = ?");
        sqlx::query(&sql).bind(visibility.as_secs() as i64).bind(id.0).execute(&self.0).await?;
        Ok(())
    }

    async fn retry_failed(&self, id: JobId) -> Result<Option<JobId>, Error> {
        let mut tx = self.0.begin().await?;
        let sql = format!(
            "insert into jobs (queue, name, payload, max_attempts, available_at, created_at)
             select queue, name, payload, max_attempts, {NOW}, {NOW} from failed_jobs where id = ?"
        );
        let res = sqlx::query(&sql).bind(id.0).execute(&mut *tx).await?;
        if res.rows_affected() == 0 {
            tx.commit().await?;
            return Ok(None);
        }
        let new_id = JobId(res.last_insert_id() as i64);
        sqlx::query("delete from failed_jobs where id = ?").bind(id.0).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Some(new_id))
    }
}

impl Queue<MysqlStore> {
    /// Enqueue inside the caller's transaction.
    pub async fn push_in_tx<J: Job>(&self, conn: &mut MySqlConnection, job: J) -> Result<JobId, Error> {
        Ok(insert(conn, &new_job(&job, Duration::ZERO)?).await?)
    }
}
