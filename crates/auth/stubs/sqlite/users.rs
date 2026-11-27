//! Published by `art publish auth`. App code: change it freely.

use db::Db;

use crate::state::Primary;

#[derive(Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub password_hash: String,
    pub remember_token: Option<String>,
}

#[derive(Clone)]
pub struct Users {
    pub db: Db<Primary>,
}

impl Users {
    pub async fn by_email(&self, email: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as("select id, email, password_hash, remember_token from users where email = ?")
            .bind(email)
            .fetch_optional(self.db.pool())
            .await
    }
}

impl auth::Provider for Users {
    type User = User;
    type Id = i64;

    async fn find(&self, id: i64) -> Result<Option<User>, auth::Error> {
        sqlx::query_as("select id, email, password_hash, remember_token from users where id = ?")
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
        sqlx::query("update users set password_hash = ? where id = ?")
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
        sqlx::query("update users set remember_token = ? where id = ?")
            .bind(token)
            .bind(id)
            .execute(self.db.pool())
            .await?;
        Ok(())
    }
}
