//! Re-encrypt a column with the newest key, for `crypt:rotate`.

use std::{borrow::Cow, fmt, future::Future};

use crate::Encrypter;

/// The column to rotate. `purpose` is what the values were sealed with;
/// `{id}` in it is replaced by each row's id (`"factor2/totp:{id}"`).
#[derive(Clone, Debug)]
pub struct Column<'a> {
    pub table: &'a str,
    pub column: &'a str,
    /// An integer key column, used to page through the table.
    pub id: &'a str,
    pub purpose: &'a str,
}

/// What a rotation did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rotated {
    /// Re-encrypted with the newest key (or, in a dry run, would be).
    pub rotated: u64,
    /// Already on the newest key, or changed by someone else meanwhile.
    pub current: u64,
    /// Did not open with any key or with this purpose: left as they are.
    pub failed: u64,
}

impl fmt::Display for Rotated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} re-encrypted, {} already current, {} could not be opened", self.rotated, self.current, self.failed)
    }
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Rows per page.
const BATCH: i64 = 500;

/// Pools that can rotate a column: Postgres, SQLite and MySQL with their features.
pub trait Rotate: Send + Sync {
    /// Re-encrypt every value of `column` sealed with an older key, page by
    /// page. Each update is conditional on the old value, so a row written
    /// meanwhile is left alone. `dry_run` only counts.
    fn rotate(&self, crypt: &Encrypter, column: &Column<'_>, dry_run: bool) -> impl Future<Output = Result<Rotated, Error>> + Send;
}

/// Table and column names come from the command line: plain identifiers only.
fn ident(s: &str) -> Result<&str, Error> {
    let ok = s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok { Ok(s) } else { Err(format!("`{s}` is not a plain table or column name").into()) }
}

/// `?` placeholders, or `$1, $2, ...` for Postgres.
fn statement(sql: String, numbered: bool) -> Cow<'static, str> {
    if !numbered {
        return Cow::Owned(sql);
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

/// One row: re-seal it if needed. `None`: it does not open.
fn reseal(crypt: &Encrypter, column: &Column<'_>, id: i64, sealed: &[u8]) -> Option<Option<Vec<u8>>> {
    if !crypt.needs_rotation(sealed) {
        return Some(None);
    }
    let purpose = column.purpose.replace("{id}", &id.to_string());
    let plain = crypt.decrypt(purpose.as_bytes(), sealed).ok()?;
    crypt.encrypt(purpose.as_bytes(), &plain).ok().map(Some)
}

macro_rules! rotate_impl {
    ($pool:ty, numbered: $numbered:expr) => {
        impl Rotate for $pool {
            async fn rotate(&self, crypt: &Encrypter, c: &Column<'_>, dry_run: bool) -> Result<Rotated, Error> {
                let (table, column, id) = (ident(c.table)?, ident(c.column)?, ident(c.id)?);
                let select = statement(format!("select {id}, {column} from {table} where {id} > ? and {column} is not null order by {id} limit ?"), $numbered);
                let update = statement(format!("update {table} set {column} = ? where {id} = ? and {column} = ?"), $numbered);
                let (mut done, mut after) = (Rotated::default(), i64::MIN);
                loop {
                    let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as(&select).bind(after).bind(BATCH).fetch_all(self).await?;
                    let Some(&(last, _)) = rows.last() else { break };
                    after = last;
                    for (row, sealed) in rows {
                        match reseal(crypt, c, row, &sealed) {
                            None => done.failed += 1,
                            Some(None) => done.current += 1,
                            Some(Some(_)) if dry_run => done.rotated += 1,
                            Some(Some(new)) => {
                                let hit = sqlx::query(&update).bind(new).bind(row).bind(&sealed).execute(self).await?.rows_affected();
                                if hit > 0 { done.rotated += 1 } else { done.current += 1 }
                            }
                        }
                    }
                }
                Ok(done)
            }
        }
    };
}

#[cfg(feature = "postgres")]
rotate_impl!(sqlx::PgPool, numbered: true);
#[cfg(feature = "sqlite")]
rotate_impl!(sqlx::SqlitePool, numbered: false);
#[cfg(feature = "mysql")]
rotate_impl!(sqlx::MySqlPool, numbered: false);

#[cfg(test)]
mod tests {
    #[test]
    fn only_plain_identifiers() {
        assert!(super::ident("totp_secrets").is_ok());
        for bad in ["", "1a", "a;drop table x", "a b", "\"x\"", "a.b"] {
            assert!(super::ident(bad).is_err(), "{bad}");
        }
    }
}
