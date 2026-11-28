//! Published by `art publish auth`. App code: change it freely.

use db::{Db, Uuid};
use serde::Serialize;
use ts_rs::TS;

use crate::state::Primary;

/// Serializes as `{ "uuid", "name", "email" }`: the internal `id` and the
/// secrets never leave the app.
#[derive(Clone, Serialize, TS, sqlx::FromRow)]
#[ts(export)]
pub struct User {
    #[serde(skip)]
    pub id: i64,
    /// Names the user in URLs and JSON.
    pub uuid: Uuid<User>,
    /// Optional: the registration form may leave it out.
    pub name: Option<String>,
    pub email: String,
    /// `None` for accounts made through social login.
    #[serde(skip)]
    pub password_hash: Option<String>,
    #[serde(skip)]
    pub remember_token: Option<String>,
    /// Unix seconds; `None` until the address is verified (`auth::Verified<User>`).
    #[ts(type = "number | null")]
    pub email_verified_at: Option<i64>,
}

/// An API token as its owner sees it, for token management: named by
/// `uuid` in URLs, never by `id` or the token itself.
#[derive(Debug, Serialize, TS, sqlx::FromRow)]
#[ts(export)]
pub struct ApiToken {
    pub uuid: Uuid<ApiToken>,
    pub name: String,
    /// Unix seconds; `None` for a token that does not expire.
    #[ts(type = "number | null")]
    pub expires_at: Option<i64>,
}

/// Where sessions live, for "Browsers & devices" (`session::Devices`).
pub type Sessions = session::MysqlStore;

#[derive(Clone)]
pub struct UserStore {
    pub db: Db<Primary>,
}

impl UserStore {
    /// The sessions table, to list and end a user's sessions.
    pub fn sessions(&self) -> Sessions {
        Sessions::new(self.db.pool().clone())
    }

    /// The user's API tokens, oldest first.
    pub async fn tokens(&self, user_id: i64) -> Result<Vec<ApiToken>, sqlx::Error> {
        sqlx::query_as("select uuid, name, expires_at from api_tokens where user_id = ? order by id")
            .bind(user_id)
            .fetch_all(self.db.pool())
            .await
    }

    /// Revoke one of the user's tokens by uuid. `false` if it is not theirs.
    pub async fn delete_token_by_uuid(&self, user_id: i64, token: Uuid<ApiToken>) -> Result<bool, sqlx::Error> {
        let done = sqlx::query("delete from api_tokens where user_id = ? and uuid = ?")
            .bind(user_id)
            .bind(token)
            .execute(self.db.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Insert a user and read it back. The unique index on `email` rejects duplicates.
    pub async fn create(&self, name: Option<&str>, email: &str, password_hash: &str) -> Result<User, sqlx::Error> {
        sqlx::query("insert into users (uuid, name, email, password_hash) values (?, ?, ?, ?)")
            .bind(Uuid::<User>::new())
            .bind(name)
            .bind(email)
            .bind(password_hash)
            .execute(self.db.pool())
            .await?;
        self.by_email(email).await?.ok_or(sqlx::Error::RowNotFound)
    }

    /// The user a URL names, e.g. `/teams/{team}/members/{user}`.
    pub async fn by_uuid(&self, uuid: Uuid<User>) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as("select id, uuid, name, email, password_hash, remember_token, email_verified_at from users where uuid = ?")
            .bind(uuid)
            .fetch_optional(self.db.pool())
            .await
    }

    pub async fn by_email(&self, email: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as("select id, uuid, name, email, password_hash, remember_token, email_verified_at from users where email = ?")
            .bind(email)
            .fetch_optional(self.db.pool())
            .await
    }
}

impl auth::Provider for UserStore {
    type User = User;
    type Id = i64;

    async fn find(&self, id: i64) -> Result<Option<User>, auth::Error> {
        sqlx::query_as("select id, uuid, name, email, password_hash, remember_token, email_verified_at from users where id = ?")
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(Into::into)
    }

    fn id(&self, user: &User) -> i64 {
        user.id
    }
}

/// Login with email and password: `auth.attempt(&users, email, password, remember)`.
impl auth::Credentials for UserStore {
    async fn by_login(&self, email: &str) -> Result<Option<User>, auth::Error> {
        Ok(self.by_email(email).await?)
    }

    fn password_hash(&self, user: &User) -> Option<String> {
        user.password_hash.clone()
    }

    async fn set_password_hash(&self, id: i64, hash: String) -> Result<(), auth::Error> {
        sqlx::query("update users set password_hash = ? where id = ?")
            .bind(hash)
            .bind(id)
            .execute(self.db.pool())
            .await?;
        Ok(())
    }
}

/// "Remember me": enable with `auth::layer(users).remember(codec)`.
impl auth::Remember for UserStore {
    fn remember_token(&self, user: &User) -> Option<String> {
        user.remember_token.clone()
    }

    async fn set_remember_token(&self, id: i64, token: Option<String>) -> Result<(), auth::Error> {
        sqlx::query("update users set remember_token = ? where id = ?")
            .bind(token)
            .bind(id)
            .execute(self.db.pool())
            .await?;
        Ok(())
    }
}

/// API tokens: enable with `.tokens()` on the auth layer, issue with
/// `auth::issue_token(&users, &user, "cli", Some(ttl))`.
impl auth::Tokens for UserStore {
    async fn by_token_hash(&self, hash: &str) -> Result<Option<(User, Option<i64>)>, auth::Error> {
        let row: Option<(i64, Option<i64>)> = sqlx::query_as("select user_id, expires_at from api_tokens where token_hash = ?")
            .bind(hash)
            .fetch_optional(self.db.pool())
            .await?;
        let Some((id, expires_at)) = row else { return Ok(None) };
        Ok(auth::Provider::find(self, id).await?.map(|user| (user, expires_at)))
    }

    async fn store_token(&self, id: i64, token: auth::NewToken) -> Result<(), auth::Error> {
        sqlx::query("insert into api_tokens (uuid, user_id, name, token_hash, expires_at) values (?, ?, ?, ?, ?)")
            .bind(Uuid::<ApiToken>::new())
            .bind(id)
            .bind(token.name)
            .bind(token.hash)
            .bind(token.expires_at)
            .execute(self.db.pool())
            .await?;
        Ok(())
    }

    async fn delete_token(&self, hash: &str) -> Result<(), auth::Error> {
        sqlx::query("delete from api_tokens where token_hash = ?").bind(hash).execute(self.db.pool()).await?;
        Ok(())
    }
}

/// "Forgot password": `auth::reset_link_token` / `auth::reset_password`.
impl auth::Resets for UserStore {
    async fn store_reset(&self, id: i64, hash: String, created_at: i64) -> Result<(), auth::Error> {
        let mut tx = self.db.pool().begin().await?;
        sqlx::query("delete from password_resets where user_id = ?").bind(id).execute(&mut *tx).await?;
        sqlx::query("insert into password_resets (user_id, token_hash, created_at) values (?, ?, ?)")
            .bind(id)
            .bind(hash)
            .bind(created_at)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn find_reset(&self, id: i64) -> Result<Option<(String, i64)>, auth::Error> {
        Ok(sqlx::query_as("select token_hash, created_at from password_resets where user_id = ?")
            .bind(id)
            .fetch_optional(self.db.pool())
            .await?)
    }

    async fn delete_reset(&self, id: i64) -> Result<(), auth::Error> {
        sqlx::query("delete from password_resets where user_id = ?").bind(id).execute(self.db.pool()).await?;
        Ok(())
    }
}

/// Email verification, also for changing the address: `auth::verification_token` /
/// `auth.verify_email`; `.verification()` on the auth layer for `auth::Verified<User>`.
impl auth::Verifications for UserStore {
    fn email(&self, user: &User) -> String {
        user.email.clone()
    }

    fn email_verified_at(&self, user: &User) -> Option<i64> {
        user.email_verified_at
    }

    async fn store_verification(&self, link: auth::NewVerification<i64>) -> Result<(), auth::Error> {
        let mut tx = self.db.pool().begin().await?;
        // One open link per user: a new one replaces any older one.
        sqlx::query("delete from email_verifications where user_id = ? and verified_at is null").bind(link.user).execute(&mut *tx).await?;
        sqlx::query("insert into email_verifications (user_id, email, token_hash, expires_at, created_at) values (?, ?, ?, ?, ?)")
            .bind(link.user)
            .bind(link.email)
            .bind(link.token_hash)
            .bind(link.expires_at)
            .bind(link.created_at)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn last_verification_at(&self, id: i64) -> Result<Option<i64>, auth::Error> {
        let (at,): (Option<i64>,) = sqlx::query_as("select max(created_at) from email_verifications where user_id = ?").bind(id).fetch_one(self.db.pool()).await?;
        Ok(at)
    }

    async fn verification_by_token(&self, hash: &[u8]) -> Result<Option<auth::Verification<i64>>, auth::Error> {
        let row: Option<(i64, i64, String, i64, Option<i64>)> =
            sqlx::query_as("select id, user_id, email, expires_at, verified_at from email_verifications where token_hash = ?")
                .bind(hash)
                .fetch_optional(self.db.pool())
                .await?;
        Ok(row.map(|(id, user, email, expires_at, verified_at)| auth::Verification { id, user, email, expires_at, verified_at }))
    }

    async fn complete_verification(&self, link: i64, user: i64, email: &str, now: i64) -> Result<bool, auth::Error> {
        let mut tx = self.db.pool().begin().await?;
        let used = sqlx::query("update email_verifications set verified_at = ? where id = ? and verified_at is null")
            .bind(now)
            .bind(link)
            .execute(&mut *tx)
            .await?;
        if used.rows_affected() == 0 {
            return Ok(false);
        }
        sqlx::query("update users set email = ?, email_verified_at = ? where id = ?").bind(email).bind(now).bind(user).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }
}
