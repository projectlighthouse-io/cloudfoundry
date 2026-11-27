//! SQL stores. The three drivers run the same statements; only the
//! placeholders and the way to read a new row's id differ, so one macro
//! writes all three. Tables: `art publish teams [--driver sqlite|mysql]`.

use std::borrow::Cow;

use crate::{Error, Invitation, Member, Org, Role, Target, Team};

fn failed(e: sqlx::Error) -> Error {
    Error::Store(Box::new(e))
}

/// A unique violation on `teams (organization_id, name)` is a taken name.
fn named(e: sqlx::Error) -> Error {
    if e.as_database_error().is_some_and(|d| d.is_unique_violation()) { Error::NameTaken } else { failed(e) }
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

/// A stored role key, or `None` (logged) for one the app no longer defines:
/// that membership grants nothing.
fn role<R: Role>(key: &str) -> Option<R> {
    let role = R::from_key(key);
    if role.is_none() {
        tracing::error!("teams: stored role `{key}` is not a role of the app; ignoring it");
    }
    role
}

fn members<R: Role>(rows: Vec<(i64, String)>) -> Vec<Member<R>> {
    rows.into_iter().filter_map(|(user_id, key)| Some(Member { user_id, role: role(&key)? })).collect()
}

type InvitationRow = (i64, String, i64, String, String, i64, i64);

fn invitation<R: Role, O: Role>((id, scope, scope_id, email, key, invited_by, expires_at): InvitationRow) -> Option<Invitation<R, O>> {
    let target = match scope.as_str() {
        "team" => Target::Team { team: scope_id, role: role(&key)? },
        "org" => Target::Org { org: scope_id, role: role(&key)? },
        _ => return None,
    };
    Some(Invitation { id, email, target, invited_by, expires_at })
}

fn team((id, org_id, name): (i64, Option<i64>, String)) -> Team {
    Team { id, org_id, name }
}

macro_rules! sql_store {
    ($(#[$doc:meta])* $name:ident, $pool:ty, numbered: $numbered:expr, last_id: $last_id:literal) => {
        $(#[$doc])*
        pub struct $name<R, O> {
            pool: $pool,
            roles: std::marker::PhantomData<fn() -> (R, O)>,
        }

        impl<R, O> Clone for $name<R, O> {
            fn clone(&self) -> Self {
                Self { pool: self.pool.clone(), roles: std::marker::PhantomData }
            }
        }

        impl<R, O> $name<R, O> {
            pub fn new(pool: $pool) -> Self {
                Self { pool, roles: std::marker::PhantomData }
            }
        }

        impl<R: Role, O: crate::OrgRole> crate::Store for $name<R, O> {
            type Role = R;
            type OrgRole = O;

            async fn create_org(&self, owner: i64, name: &str) -> Result<Org, Error> {
                let mut tx = self.pool.begin().await.map_err(failed)?;
                sqlx::query(&statement("insert into organizations (name) values (?)", $numbered)).bind(name).execute(&mut *tx).await.map_err(failed)?;
                let (id,): (i64,) = sqlx::query_as($last_id).fetch_one(&mut *tx).await.map_err(failed)?;
                sqlx::query(&statement("insert into organization_members (organization_id, user_id, role) values (?, ?, ?)", $numbered))
                    .bind(id)
                    .bind(owner)
                    .bind(O::OWNER.key())
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                tx.commit().await.map_err(failed)?;
                Ok(Org { id, name: name.to_owned() })
            }

            async fn org(&self, id: i64) -> Result<Option<Org>, Error> {
                let row: Option<(i64, String)> = sqlx::query_as(&statement("select id, name from organizations where id = ?", $numbered))
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(row.map(|(id, name)| Org { id, name }))
            }

            async fn rename_org(&self, id: i64, name: &str) -> Result<(), Error> {
                sqlx::query(&statement("update organizations set name = ? where id = ?", $numbered)).bind(name).bind(id).execute(&self.pool).await.map_err(failed)?;
                Ok(())
            }

            async fn delete_org(&self, id: i64) -> Result<(), Error> {
                let mut tx = self.pool.begin().await.map_err(failed)?;
                for sql in [
                    "delete from team_invitations where scope = 'team' and scope_id in (select id from teams where organization_id = ?)",
                    "delete from team_invitations where scope = 'org' and scope_id = ?",
                    "delete from team_members where team_id in (select id from teams where organization_id = ?)",
                    "delete from teams where organization_id = ?",
                    "delete from organization_members where organization_id = ?",
                    "delete from organizations where id = ?",
                ] {
                    sqlx::query(&statement(sql, $numbered)).bind(id).execute(&mut *tx).await.map_err(failed)?;
                }
                tx.commit().await.map_err(failed)
            }

            async fn org_role(&self, org: i64, user: i64) -> Result<Option<O>, Error> {
                let row: Option<(String,)> = sqlx::query_as(&statement("select role from organization_members where organization_id = ? and user_id = ?", $numbered))
                    .bind(org)
                    .bind(user)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(row.and_then(|(key,)| role(&key)))
            }

            async fn set_org_role(&self, org: i64, user: i64, r: O) -> Result<(), Error> {
                let mut tx = self.pool.begin().await.map_err(failed)?;
                sqlx::query(&statement("delete from organization_members where organization_id = ? and user_id = ?", $numbered))
                    .bind(org)
                    .bind(user)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                sqlx::query(&statement("insert into organization_members (organization_id, user_id, role) values (?, ?, ?)", $numbered))
                    .bind(org)
                    .bind(user)
                    .bind(r.key())
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                tx.commit().await.map_err(failed)
            }

            async fn remove_org_member(&self, org: i64, user: i64) -> Result<(), Error> {
                let mut tx = self.pool.begin().await.map_err(failed)?;
                sqlx::query(&statement("delete from team_members where user_id = ? and team_id in (select id from teams where organization_id = ?)", $numbered))
                    .bind(user)
                    .bind(org)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                sqlx::query(&statement("delete from organization_members where organization_id = ? and user_id = ?", $numbered))
                    .bind(org)
                    .bind(user)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                tx.commit().await.map_err(failed)
            }

            async fn org_members(&self, org: i64) -> Result<Vec<Member<O>>, Error> {
                let rows = sqlx::query_as(&statement("select user_id, role from organization_members where organization_id = ? order by user_id", $numbered))
                    .bind(org)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(members(rows))
            }

            async fn orgs_of(&self, user: i64) -> Result<Vec<(Org, O)>, Error> {
                let rows: Vec<(i64, String, String)> = sqlx::query_as(&statement(
                    "select o.id, o.name, m.role from organizations o join organization_members m on m.organization_id = o.id where m.user_id = ? order by o.id",
                    $numbered,
                ))
                .bind(user)
                .fetch_all(&self.pool)
                .await
                .map_err(failed)?;
                Ok(rows.into_iter().filter_map(|(id, name, key)| Some((Org { id, name }, role(&key)?))).collect())
            }

            async fn create_team(&self, owner: i64, org: Option<i64>, name: &str) -> Result<Team, Error> {
                let mut tx = self.pool.begin().await.map_err(failed)?;
                sqlx::query(&statement("insert into teams (organization_id, name) values (?, ?)", $numbered))
                    .bind(org)
                    .bind(name)
                    .execute(&mut *tx)
                    .await
                    .map_err(named)?;
                let (id,): (i64,) = sqlx::query_as($last_id).fetch_one(&mut *tx).await.map_err(failed)?;
                sqlx::query(&statement("insert into team_members (team_id, user_id, role) values (?, ?, ?)", $numbered))
                    .bind(id)
                    .bind(owner)
                    .bind(R::OWNER.key())
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                tx.commit().await.map_err(failed)?;
                Ok(Team { id, org_id: org, name: name.to_owned() })
            }

            async fn team(&self, id: i64) -> Result<Option<Team>, Error> {
                let row = sqlx::query_as(&statement("select id, organization_id, name from teams where id = ?", $numbered))
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(row.map(team))
            }

            async fn rename_team(&self, id: i64, name: &str) -> Result<(), Error> {
                sqlx::query(&statement("update teams set name = ? where id = ?", $numbered)).bind(name).bind(id).execute(&self.pool).await.map_err(named)?;
                Ok(())
            }

            async fn delete_team(&self, id: i64) -> Result<(), Error> {
                let mut tx = self.pool.begin().await.map_err(failed)?;
                for sql in [
                    "delete from team_invitations where scope = 'team' and scope_id = ?",
                    "delete from team_members where team_id = ?",
                    "delete from teams where id = ?",
                ] {
                    sqlx::query(&statement(sql, $numbered)).bind(id).execute(&mut *tx).await.map_err(failed)?;
                }
                tx.commit().await.map_err(failed)
            }

            async fn team_role(&self, team: i64, user: i64) -> Result<Option<R>, Error> {
                let row: Option<(String,)> = sqlx::query_as(&statement("select role from team_members where team_id = ? and user_id = ?", $numbered))
                    .bind(team)
                    .bind(user)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(row.and_then(|(key,)| role(&key)))
            }

            async fn set_team_role(&self, team: i64, user: i64, r: R) -> Result<(), Error> {
                let mut tx = self.pool.begin().await.map_err(failed)?;
                sqlx::query(&statement("delete from team_members where team_id = ? and user_id = ?", $numbered))
                    .bind(team)
                    .bind(user)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                sqlx::query(&statement("insert into team_members (team_id, user_id, role) values (?, ?, ?)", $numbered))
                    .bind(team)
                    .bind(user)
                    .bind(r.key())
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                tx.commit().await.map_err(failed)
            }

            async fn remove_team_member(&self, team: i64, user: i64) -> Result<(), Error> {
                sqlx::query(&statement("delete from team_members where team_id = ? and user_id = ?", $numbered))
                    .bind(team)
                    .bind(user)
                    .execute(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(())
            }

            async fn team_members(&self, team: i64) -> Result<Vec<Member<R>>, Error> {
                let rows = sqlx::query_as(&statement("select user_id, role from team_members where team_id = ? order by user_id", $numbered))
                    .bind(team)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(members(rows))
            }

            async fn teams_of(&self, user: i64) -> Result<Vec<(Team, R)>, Error> {
                let rows: Vec<(i64, Option<i64>, String, String)> = sqlx::query_as(&statement(
                    "select t.id, t.organization_id, t.name, m.role from teams t join team_members m on m.team_id = t.id where m.user_id = ? order by t.id",
                    $numbered,
                ))
                .bind(user)
                .fetch_all(&self.pool)
                .await
                .map_err(failed)?;
                Ok(rows.into_iter().filter_map(|(id, org, name, key)| Some((team((id, org, name)), role(&key)?))).collect())
            }

            async fn org_teams(&self, org: i64) -> Result<Vec<Team>, Error> {
                let rows: Vec<(i64, Option<i64>, String)> = sqlx::query_as(&statement("select id, organization_id, name from teams where organization_id = ? order by id", $numbered))
                    .bind(org)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(failed)?;
                Ok(rows.into_iter().map(team).collect())
            }

            async fn create_invitation(&self, email: &str, target: Target<R, O>, invited_by: i64, expires_at: i64, token_hash: &str) -> Result<Invitation<R, O>, Error> {
                let (scope, scope_id, key) = match target {
                    Target::Team { team, role } => ("team", team, role.key()),
                    Target::Org { org, role } => ("org", org, role.key()),
                };
                let mut tx = self.pool.begin().await.map_err(failed)?;
                sqlx::query(&statement(
                    "insert into team_invitations (scope, scope_id, email, role, token_hash, invited_by, expires_at) values (?, ?, ?, ?, ?, ?, ?)",
                    $numbered,
                ))
                .bind(scope)
                .bind(scope_id)
                .bind(email)
                .bind(key)
                .bind(token_hash)
                .bind(invited_by)
                .bind(expires_at)
                .execute(&mut *tx)
                .await
                .map_err(failed)?;
                let (id,): (i64,) = sqlx::query_as($last_id).fetch_one(&mut *tx).await.map_err(failed)?;
                tx.commit().await.map_err(failed)?;
                Ok(Invitation { id, email: email.to_owned(), target, invited_by, expires_at })
            }

            async fn invitation(&self, token_hash: &str) -> Result<Option<Invitation<R, O>>, Error> {
                let row: Option<InvitationRow> = sqlx::query_as(&statement(
                    "select id, scope, scope_id, email, role, invited_by, expires_at from team_invitations where token_hash = ?",
                    $numbered,
                ))
                .bind(token_hash)
                .fetch_optional(&self.pool)
                .await
                .map_err(failed)?;
                Ok(row.and_then(invitation))
            }

            async fn team_invitations(&self, team: i64) -> Result<Vec<Invitation<R, O>>, Error> {
                let rows: Vec<InvitationRow> = sqlx::query_as(&statement(
                    "select id, scope, scope_id, email, role, invited_by, expires_at from team_invitations where scope = 'team' and scope_id = ? order by id",
                    $numbered,
                ))
                .bind(team)
                .fetch_all(&self.pool)
                .await
                .map_err(failed)?;
                Ok(rows.into_iter().filter_map(invitation).collect())
            }

            async fn org_invitations(&self, org: i64) -> Result<Vec<Invitation<R, O>>, Error> {
                let rows: Vec<InvitationRow> = sqlx::query_as(&statement(
                    "select id, scope, scope_id, email, role, invited_by, expires_at from team_invitations where scope = 'org' and scope_id = ? order by id",
                    $numbered,
                ))
                .bind(org)
                .fetch_all(&self.pool)
                .await
                .map_err(failed)?;
                Ok(rows.into_iter().filter_map(invitation).collect())
            }

            async fn delete_invitation(&self, id: i64) -> Result<(), Error> {
                sqlx::query(&statement("delete from team_invitations where id = ?", $numbered)).bind(id).execute(&self.pool).await.map_err(failed)?;
                Ok(())
            }
        }
    };
}

#[cfg(feature = "sqlite")]
sql_store!(
    /// Teams in SQLite (`art publish teams --driver sqlite`).
    SqliteStore, sqlx::SqlitePool, numbered: false, last_id: "select last_insert_rowid()"
);

#[cfg(feature = "postgres")]
sql_store!(
    /// Teams in Postgres (`art publish teams`).
    PostgresStore, sqlx::PgPool, numbered: true, last_id: "select lastval()"
);

#[cfg(feature = "mysql")]
sql_store!(
    /// Teams in MySQL (`art publish teams --driver mysql`).
    MysqlStore, sqlx::MySqlPool, numbered: false, last_id: "select cast(last_insert_id() as signed)"
);
