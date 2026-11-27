//! Published by `art publish auth`. App code: change it freely.

use db::Db;

use crate::state::Primary;

#[derive(Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    /// Optional: the registration form may leave it out.
    #[allow(dead_code)] // read once your pages show it
    pub name: Option<String>,
    pub email: String,
    pub password_hash: String,
    pub remember_token: Option<String>,
}

#[derive(Clone)]
pub struct Users {
    pub db: Db<Primary>,
}

impl Users {
    /// Insert a user and read it back. The unique index on `email` rejects duplicates.
    pub async fn create(&self, name: Option<&str>, email: &str, password_hash: &str) -> Result<User, sqlx::Error> {
        sqlx::query("insert into users (name, email, password_hash) values ($1, $2, $3)")
            .bind(name)
            .bind(email)
            .bind(password_hash)
            .execute(self.db.pool())
            .await?;
        self.by_email(email).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn by_email(&self, email: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as("select id, name, email, password_hash, remember_token from users where email = $1")
            .bind(email)
            .fetch_optional(self.db.pool())
            .await
    }
}

impl auth::Provider for Users {
    type User = User;
    type Id = i64;

    async fn find(&self, id: i64) -> Result<Option<User>, auth::Error> {
        sqlx::query_as("select id, name, email, password_hash, remember_token from users where id = $1")
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

    fn password_hash(&self, user: &User) -> String {
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
        sqlx::query("insert into api_tokens (user_id, name, token_hash, expires_at) values ($1, $2, $3, $4)")
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
