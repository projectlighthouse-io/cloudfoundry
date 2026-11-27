//! Published by `art publish socialite`. App code: change it freely.
//!
//! How a provider's user becomes an app user. The policy: an account already
//! linked to that provider account logs in; a logged-in user links it to
//! themselves; otherwise the same email means the same account; otherwise a
//! new account without a password is created.

use socialite::SocialUser;

use crate::users::{User, Users};

/// Why a social login could not produce a user.
#[derive(Debug)]
pub enum SocialError {
    /// The provider shares no email (Reddit, X): log in first, then connect it.
    NoEmail,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for SocialError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

impl Users {
    /// The user linked to this provider account, if any.
    pub async fn by_social(&self, provider: &str, provider_id: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as(
            "select u.id, u.name, u.email, u.password_hash, u.remember_token from users u
             join social_accounts s on s.user_id = u.id where s.provider = $1 and s.provider_id = $2",
        )
        .bind(provider)
        .bind(provider_id)
        .fetch_optional(self.db.pool())
        .await
    }

    pub async fn link_social(&self, user_id: i64, provider: &str, provider_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("insert into social_accounts (user_id, provider, provider_id) values ($1, $2, $3)")
            .bind(user_id)
            .bind(provider)
            .bind(provider_id)
            .execute(self.db.pool())
            .await?;
        Ok(())
    }

    /// The account for `social`, linking or creating it as needed.
    /// `current` is the logged-in user, who gets the provider account linked.
    pub async fn find_or_create_social(&self, social: &SocialUser, current: Option<&User>) -> Result<User, SocialError> {
        if let Some(user) = self.by_social(social.provider, &social.id).await? {
            return Ok(user);
        }
        if let Some(user) = current {
            self.link_social(user.id, social.provider, &social.id).await?;
            return Ok(user.clone());
        }
        let email = social.email.as_deref().ok_or(SocialError::NoEmail)?;
        let user = match self.by_email(email).await? {
            Some(user) => user,
            None => {
                sqlx::query("insert into users (name, email, password_hash) values ($1, $2, null)")
                    .bind(social.name.as_deref())
                    .bind(email)
                    .execute(self.db.pool())
                    .await?;
                self.by_email(email).await?.ok_or(sqlx::Error::RowNotFound)?
            }
        };
        self.link_social(user.id, social.provider, &social.id).await?;
        Ok(user)
    }
}
