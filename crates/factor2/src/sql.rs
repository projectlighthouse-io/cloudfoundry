//! SQL stores. The three drivers run the same statements; only the
//! placeholders differ, so one macro writes all three. Tables:
//! `art publish factor2 [--driver sqlite|mysql]`.

use std::borrow::Cow;

use db::Uuid;

use crate::{Error, NewDevice, StoredSecret, TrustedDevice};

fn failed(e: sqlx::Error) -> Error {
    Error::other(e)
}

/// `?` placeholders, or `$1, $2, ...` for Postgres.
fn statement(sql: &'static str, numbered: bool) -> Cow<'static, str> {
    if !numbered {
        return Cow::Borrowed(sql);
    }
    let mut out = String::with_capacity(sql.len() + 8);
    let mut n = 0;
    for c in sql.chars() {
        if c == '?' {
            n += 1;
            out.push('$');
            out.push_str(&n.to_string());
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

type DeviceRow = (i64, Uuid<TrustedDevice>, i64, Option<String>, Option<String>, i64, Option<i64>, i64);

fn device((id, uuid, user_id, user_agent, ip, expires_at, last_used_at, created_at): DeviceRow) -> TrustedDevice {
    TrustedDevice { id, uuid, user_id, user_agent, ip, expires_at, last_used_at, created_at }
}

const DEVICE_COLUMNS: &str = "id, uuid, user_id, user_agent, ip, expires_at, last_used_at, created_at";

macro_rules! sql_store {
    ($(#[$doc:meta])* $name:ident, $pool:ty, numbered: $numbered:expr) => {
        $(#[$doc])*
        #[derive(Clone)]
        pub struct $name {
            pool: $pool,
        }

        impl $name {
            pub fn new(pool: $pool) -> Self {
                Self { pool }
            }
        }

        impl crate::Store for $name {
            async fn secret(&self, user: i64) -> Result<Option<StoredSecret>, Error> {
                let row: Option<(Vec<u8>, Option<i64>, Option<i64>)> =
                    sqlx::query_as(&statement("select secret_encrypted, confirmed_at, last_used_step from totp_secrets where user_id = ?", $numbered))
                        .bind(user)
                        .fetch_optional(&self.pool)
                        .await
                        .map_err(failed)?;
                Ok(row.map(|(sealed, confirmed_at, last_step)| StoredSecret { sealed, confirmed_at, last_step }))
            }

            async fn put_secret(&self, user: i64, sealed: &[u8], now: i64) -> Result<(), Error> {
                let mut tx = self.pool.begin().await.map_err(failed)?;
                sqlx::query(&statement("delete from totp_secrets where user_id = ?", $numbered)).bind(user).execute(&mut *tx).await.map_err(failed)?;
                sqlx::query(&statement("insert into totp_secrets (user_id, secret_encrypted, created_at) values (?, ?, ?)", $numbered))
                    .bind(user)
                    .bind(sealed)
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                tx.commit().await.map_err(failed)
            }

            async fn confirm_secret(&self, user: i64, now: i64, step: i64) -> Result<bool, Error> {
                let done = sqlx::query(&statement(
                    "update totp_secrets set confirmed_at = ?, last_used_step = ? where user_id = ? and confirmed_at is null",
                    $numbered,
                ))
                .bind(now)
                .bind(step)
                .bind(user)
                .execute(&self.pool)
                .await
                .map_err(failed)?;
                Ok(done.rows_affected() > 0)
            }

            async fn use_step(&self, user: i64, step: i64) -> Result<bool, Error> {
                let done = sqlx::query(&statement(
                    "update totp_secrets set last_used_step = ? where user_id = ? and confirmed_at is not null and (last_used_step is null or last_used_step < ?)",
                    $numbered,
                ))
                .bind(step)
                .bind(user)
                .bind(step)
                .execute(&self.pool)
                .await
                .map_err(failed)?;
                Ok(done.rows_affected() > 0)
            }

            async fn delete_secret(&self, user: i64) -> Result<(), Error> {
                sqlx::query(&statement("delete from totp_secrets where user_id = ?", $numbered)).bind(user).execute(&self.pool).await.map_err(failed)?;
                Ok(())
            }

            async fn replace_backup_codes(&self, user: i64, hashes: &[Vec<u8>]) -> Result<(), Error> {
                let mut tx = self.pool.begin().await.map_err(failed)?;
                sqlx::query(&statement("delete from backup_codes where user_id = ?", $numbered)).bind(user).execute(&mut *tx).await.map_err(failed)?;
                for hash in hashes {
                    sqlx::query(&statement("insert into backup_codes (user_id, code_hash) values (?, ?)", $numbered))
                        .bind(user)
                        .bind(hash)
                        .execute(&mut *tx)
                        .await
                        .map_err(failed)?;
                }
                tx.commit().await.map_err(failed)
            }

            async fn use_backup_code(&self, user: i64, hash: &[u8], now: i64) -> Result<bool, Error> {
                let done = sqlx::query(&statement("update backup_codes set used_at = ? where user_id = ? and code_hash = ? and used_at is null", $numbered))
                    .bind(now)
                    .bind(user)
                    .bind(hash)
                    .execute(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(done.rows_affected() > 0)
            }

            async fn unused_backup_codes(&self, user: i64) -> Result<usize, Error> {
                let (n,): (i64,) = sqlx::query_as(&statement("select count(*) from backup_codes where user_id = ? and used_at is null", $numbered))
                    .bind(user)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(n.max(0) as usize)
            }

            async fn add_device(&self, d: NewDevice) -> Result<(), Error> {
                sqlx::query(&statement(
                    "insert into trusted_devices (uuid, user_id, token_hash, user_agent, ip, expires_at, created_at) values (?, ?, ?, ?, ?, ?, ?)",
                    $numbered,
                ))
                .bind(d.uuid)
                .bind(d.user_id)
                .bind(d.token_hash)
                .bind(d.user_agent)
                .bind(d.ip)
                .bind(d.expires_at)
                .bind(d.created_at)
                .execute(&self.pool)
                .await
                .map_err(failed)?;
                Ok(())
            }

            async fn device_by_token(&self, token_hash: &[u8]) -> Result<Option<TrustedDevice>, Error> {
                let sql = format!("select {DEVICE_COLUMNS} from trusted_devices where token_hash = {}", if $numbered { "$1" } else { "?" });
                let row: Option<DeviceRow> = sqlx::query_as(&sql).bind(token_hash).fetch_optional(&self.pool).await.map_err(failed)?;
                Ok(row.map(device))
            }

            async fn touch_device(&self, id: i64, now: i64) -> Result<(), Error> {
                sqlx::query(&statement("update trusted_devices set last_used_at = ? where id = ?", $numbered))
                    .bind(now)
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(())
            }

            async fn devices(&self, user: i64) -> Result<Vec<TrustedDevice>, Error> {
                let sql = format!("select {DEVICE_COLUMNS} from trusted_devices where user_id = {} order by id desc", if $numbered { "$1" } else { "?" });
                let rows: Vec<DeviceRow> = sqlx::query_as(&sql).bind(user).fetch_all(&self.pool).await.map_err(failed)?;
                Ok(rows.into_iter().map(device).collect())
            }

            async fn delete_device(&self, user: i64, uuid: Uuid<TrustedDevice>) -> Result<bool, Error> {
                let done = sqlx::query(&statement("delete from trusted_devices where user_id = ? and uuid = ?", $numbered))
                    .bind(user)
                    .bind(uuid)
                    .execute(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(done.rows_affected() > 0)
            }

            async fn delete_all(&self, user: i64) -> Result<(), Error> {
                let mut tx = self.pool.begin().await.map_err(failed)?;
                for sql in ["delete from trusted_devices where user_id = ?", "delete from backup_codes where user_id = ?", "delete from totp_secrets where user_id = ?"] {
                    sqlx::query(&statement(sql, $numbered)).bind(user).execute(&mut *tx).await.map_err(failed)?;
                }
                tx.commit().await.map_err(failed)
            }
        }
    };
}

#[cfg(feature = "sqlite")]
sql_store!(
    /// Two-factor data in SQLite (`art publish factor2 --driver sqlite`).
    SqliteStore, sqlx::SqlitePool, numbered: false
);

#[cfg(feature = "postgres")]
sql_store!(
    /// Two-factor data in Postgres (`art publish factor2`).
    PostgresStore, sqlx::PgPool, numbered: true
);

#[cfg(feature = "mysql")]
sql_store!(
    /// Two-factor data in MySQL (`art publish factor2 --driver mysql`).
    MysqlStore, sqlx::MySqlPool, numbered: false
);
