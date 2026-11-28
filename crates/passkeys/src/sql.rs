//! SQL stores. The three drivers run the same statements; only the
//! placeholders differ, so one macro writes all three. Table:
//! `art publish passkeys [--driver sqlite|mysql]`.

use std::borrow::Cow;

use db::Uuid;
use sqlx::types::Json;

use crate::{Credential, Error, NewPasskey, Passkey};

fn failed(e: sqlx::Error) -> Error {
    Error::other(e)
}

/// `?` placeholders, or `$1, $2, ...` for Postgres.
fn statement(sql: &str, numbered: bool) -> Cow<'_, str> {
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

type Row = (i64, Uuid<Passkey>, i64, String, Vec<u8>, uuid::Uuid, Json<Credential>, Option<i64>, i64);

fn passkey((id, public_id, user_id, name, credential_id, user_handle, Json(credential), last_used_at, created_at): Row) -> Passkey {
    Passkey { id, public_id, user_id, name, credential_id, user_handle, credential, last_used_at, created_at }
}

const SELECT: &str = "select id, public_id, user_id, name, credential_id, user_handle, credential, last_used_at, created_at from passkeys";

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
            async fn add(&self, p: NewPasskey) -> Result<(), Error> {
                sqlx::query(&statement(
                    "insert into passkeys (public_id, user_id, name, credential_id, user_handle, credential, created_at) values (?, ?, ?, ?, ?, ?, ?)",
                    $numbered,
                ))
                .bind(p.public_id)
                .bind(p.user_id)
                .bind(p.name)
                .bind(p.credential_id)
                .bind(p.user_handle)
                .bind(Json(p.credential))
                .bind(p.created_at)
                .execute(&self.pool)
                .await
                .map_err(failed)?;
                Ok(())
            }

            async fn passkeys(&self, user: i64) -> Result<Vec<Passkey>, Error> {
                let rows: Vec<Row> = sqlx::query_as(&statement(&format!("{SELECT} where user_id = ? order by id desc"), $numbered))
                    .bind(user)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(rows.into_iter().map(passkey).collect())
            }

            async fn by_credential_id(&self, credential_id: &[u8]) -> Result<Option<Passkey>, Error> {
                let row: Option<Row> = sqlx::query_as(&statement(&format!("{SELECT} where credential_id = ?"), $numbered))
                    .bind(credential_id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(row.map(passkey))
            }

            async fn used(&self, id: i64, credential: &Credential, now: i64) -> Result<(), Error> {
                sqlx::query(&statement("update passkeys set credential = ?, last_used_at = ? where id = ?", $numbered))
                    .bind(Json(credential))
                    .bind(now)
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(())
            }

            async fn rename(&self, user: i64, passkey: Uuid<Passkey>, name: &str) -> Result<bool, Error> {
                // MySQL counts changed rows: renaming to the same name must still find the row.
                let found: Option<(i64,)> = sqlx::query_as(&statement("select id from passkeys where user_id = ? and public_id = ?", $numbered))
                    .bind(user)
                    .bind(passkey)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(failed)?;
                let Some((id,)) = found else { return Ok(false) };
                sqlx::query(&statement("update passkeys set name = ? where id = ?", $numbered))
                    .bind(name)
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(true)
            }

            async fn delete(&self, user: i64, passkey: Uuid<Passkey>) -> Result<bool, Error> {
                let done = sqlx::query(&statement("delete from passkeys where user_id = ? and public_id = ?", $numbered))
                    .bind(user)
                    .bind(passkey)
                    .execute(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(done.rows_affected() > 0)
            }

            async fn delete_all(&self, user: i64) -> Result<(), Error> {
                sqlx::query(&statement("delete from passkeys where user_id = ?", $numbered)).bind(user).execute(&self.pool).await.map_err(failed)?;
                Ok(())
            }
        }
    };
}

#[cfg(feature = "sqlite")]
sql_store!(
    /// Passkeys in SQLite (`art publish passkeys --driver sqlite`).
    SqliteStore, sqlx::SqlitePool, numbered: false
);

#[cfg(feature = "postgres")]
sql_store!(
    /// Passkeys in Postgres (`art publish passkeys`).
    PostgresStore, sqlx::PgPool, numbered: true
);

#[cfg(feature = "mysql")]
sql_store!(
    /// Passkeys in MySQL (`art publish passkeys --driver mysql`).
    MysqlStore, sqlx::MySqlPool, numbered: false
);
