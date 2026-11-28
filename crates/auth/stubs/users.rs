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

#[derive(Clone)]
pub struct Users {
    pub db: Db<Primary>,
}

impl Users {
    /// The user's API tokens, oldest first.
    pub async fn tokens(&self, user_id: i64) -> Result<Vec<ApiToken>, sqlx::Error> {
        sqlx::query_as("select uuid, name, expires_at from api_tokens where user_id = $1 order by id")
            .bind(user_id)
            .fetch_all(self.db.pool())
            .await
    }

    /// Revoke one of the user's tokens by uuid. `false` if it is not theirs.
    pub async fn delete_token_by_uuid(&self, user_id: i64, token: Uuid<ApiToken>) -> Result<bool, sqlx::Error> {
        let done = sqlx::query("delete from api_tokens where user_id = $1 and uuid = $2")
            .bind(user_id)
            .bind(token)
            .execute(self.db.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Insert a user and read it back. The unique index on `email` rejects duplicates.
    pub async fn create(&self, name: Option<&str>, email: &str, password_hash: &str) -> Result<User, sqlx::Error> {
        sqlx::query("insert into users (uuid, name, email, password_hash) values ($1, $2, $3, $4)")
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
        sqlx::query_as("select id, uuid, name, email, password_hash, remember_token from users where uuid = $1")
            .bind(uuid)
            .fetch_optional(self.db.pool())
            .await
    }

    pub async fn by_email(&self, email: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as("select id, uuid, name, email, password_hash, remember_token from users where email = $1")
            .bind(email)
            .fetch_optional(self.db.pool())
            .await
    }
}

impl auth::Provider for Users {
    type User = User;
    type Id = i64;

    async fn find(&self, id: i64) -> Result<Option<User>, auth::Error> {
        sqlx::query_as("select id, uuid, name, email, password_hash, remember_token from users where id = $1")
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
impl auth::Credentials for Users {
    async fn by_login(&self, email: &str) -> Result<Option<User>, auth::Error> {
        Ok(self.by_email(email).await?)
    }

    fn password_hash(&self, user: &User) -> Option<String> {
        user.password_hash.clone()
    }

    async fn set_password_hash(&self, id: i64, hash: String) -> Result<(), auth::Error> {
        sqlx::query("update users set password_hash = $1 where id = $2")
            .bind(hash)
            .bind(id)
            .execute(self.db.pool())
            .await?;
        Ok(())
    }
}

/// "Remember me": enable with `auth::layer(users).remember(codec)`.
impl auth::Remember for Users {
    fn remember_token(&self, user: &User) -> Option<String> {
        user.remember_token.clone()
    }

    async fn set_remember_token(&self, id: i64, token: Option<String>) -> Result<(), auth::Error> {
        sqlx::query("update users set remember_token = $1 where id = $2")
            .bind(token)
            .bind(id)
            .execute(self.db.pool())
            .await?;
        Ok(())
    }
}

/// API tokens: enable with `.tokens()` on the auth layer, issue with
/// `auth::issue_token(&users, &user, "cli", Some(ttl))`.
impl auth::Tokens for Users {
    async fn by_token_hash(&self, hash: &str) -> Result<Option<(User, Option<i64>)>, auth::Error> {
        let row: Option<(i64, Option<i64>)> = sqlx::query_as("select user_id, expires_at from api_tokens where token_hash = $1")
            .bind(hash)
            .fetch_optional(self.db.pool())
            .await?;
        let Some((id, expires_at)) = row else { return Ok(None) };
        Ok(auth::Provider::find(self, id).await?.map(|user| (user, expires_at)))
    }

    async fn store_token(&self, id: i64, token: auth::NewToken) -> Result<(), auth::Error> {
        sqlx::query("insert into api_tokens (uuid, user_id, name, token_hash, expires_at) values ($1, $2, $3, $4, $5)")
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
        sqlx::query("delete from api_tokens where token_hash = $1").bind(hash).execute(self.db.pool()).await?;
        Ok(())
    }
}

/// "Forgot password": `auth::reset_link_token` / `auth::reset_password`.
impl auth::Resets for Users {
    async fn store_reset(&self, id: i64, hash: String, created_at: i64) -> Result<(), auth::Error> {
        let mut tx = self.db.pool().begin().await?;
        sqlx::query("delete from password_resets where user_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("insert into password_resets (user_id, token_hash, created_at) values ($1, $2, $3)")
            .bind(id)
            .bind(hash)
            .bind(created_at)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn find_reset(&self, id: i64) -> Result<Option<(String, i64)>, auth::Error> {
        Ok(sqlx::query_as("select token_hash, created_at from password_resets where user_id = $1")
            .bind(id)
            .fetch_optional(self.db.pool())
            .await?)
    }

    async fn delete_reset(&self, id: i64) -> Result<(), auth::Error> {
        sqlx::query("delete from password_resets where user_id = $1").bind(id).execute(self.db.pool()).await?;
        Ok(())
    }
}
