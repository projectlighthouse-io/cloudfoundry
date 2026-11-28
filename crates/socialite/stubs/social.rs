//! Published by `art publish socialite`. App code: change it freely.
//!
//! How a provider's user becomes an app user. The policy: an account already
//! linked to that provider account logs in; a logged-in user links it to
//! themselves; otherwise the same *verified* email means the same account;
//! otherwise a new account without a password is created.

use db::Uuid;
use serde::Serialize;
use socialite::SocialUser;
use ts_rs::TS;

/// Link or create accounts by email even when the provider does not vouch
/// for it. Leave this `false`: with `true`, anyone who can get a provider to
/// report your users' email addresses, unverified, can sign in as them.
pub const TRUST_UNVERIFIED_EMAILS: bool = false;

use crate::users::{User, UserStore};

/// Why a social login could not produce a user.
#[derive(Debug)]
pub enum SocialError {
    /// The provider shares no email (Reddit, X): log in first, then connect it.
    NoEmail,
    /// The provider does not vouch for the email, so it cannot prove who this
    /// is: log in first, then connect it.
    UnverifiedEmail,
    Db(sqlx::Error),
}

/// A provider account linked to a user, for "Connected accounts".
#[derive(Debug, Serialize, TS, sqlx::FromRow)]
#[ts(export)]
pub struct SocialAccount {
    pub provider: String,
    /// What the provider last reported.
    pub email: Option<String>,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    /// Unix seconds of the last login through it.
    #[ts(type = "number | null")]
    pub last_used_at: Option<i64>,
}

/// Why [`UserStore::unlink_social`] did not unlink.
#[derive(Debug)]
pub enum UnlinkError {
    /// The user has no account with this provider.
    NotLinked,
    /// No password and no other provider: unlinking would lock them out.
    /// Set a password first (or add another way to sign in).
    LastSignIn,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for UnlinkError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl From<sqlx::Error> for SocialError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

impl UserStore {
    /// The user linked to this provider account, if any.
    pub async fn by_social(&self, provider: &str, provider_id: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as(
            "select u.id, u.uuid, u.name, u.email, u.password_hash, u.remember_token, u.email_verified_at from users u
             join social_accounts s on s.user_id = u.id where s.provider = $1 and s.provider_id = $2",
        )
        .bind(provider)
        .bind(provider_id)
        .fetch_optional(self.db.pool())
        .await
    }

    pub async fn link_social(&self, user_id: i64, social: &SocialUser) -> Result<(), sqlx::Error> {
        sqlx::query(
            "insert into social_accounts (user_id, provider, provider_id, email, username, avatar_url, last_used_at) values ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(user_id)
        .bind(social.provider)
        .bind(&social.id)
        .bind(social.email.as_deref())
        .bind(social.username.as_deref())
        .bind(social.avatar.as_deref())
        .bind(now())
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    /// Each login refreshes what the provider reports and `last_used_at`.
    pub async fn touch_social(&self, social: &SocialUser) -> Result<(), sqlx::Error> {
        sqlx::query("update social_accounts set email = $1, username = $2, avatar_url = $3, last_used_at = $4 where provider = $5 and provider_id = $6")
            .bind(social.email.as_deref())
            .bind(social.username.as_deref())
            .bind(social.avatar.as_deref())
            .bind(now())
            .bind(social.provider)
            .bind(&social.id)
            .execute(self.db.pool())
            .await?;
        Ok(())
    }

    /// The user's linked provider accounts, by provider.
    pub async fn social_accounts(&self, user_id: i64) -> Result<Vec<SocialAccount>, sqlx::Error> {
        sqlx::query_as("select provider, email, username, avatar_url, last_used_at from social_accounts where user_id = $1 order by provider")
            .bind(user_id)
            .fetch_all(self.db.pool())
            .await
    }

    /// Unlink `provider` from `user`, unless it is their last way to sign in
    /// (no password, no other provider). Using passkeys too? Count them as a
    /// way in here as well (`passkeys(&app).passkeys(user.id)`).
    pub async fn unlink_social(&self, user: &User, provider: &str) -> Result<(), UnlinkError> {
        let accounts = self.social_accounts(user.id).await?;
        if !accounts.iter().any(|a| a.provider == provider) {
            return Err(UnlinkError::NotLinked);
        }
        if user.password_hash.is_none() && accounts.len() == 1 {
            return Err(UnlinkError::LastSignIn);
        }
        sqlx::query("delete from social_accounts where user_id = $1 and provider = $2").bind(user.id).bind(provider).execute(self.db.pool()).await?;
        Ok(())
    }

    /// The account for `social`, linking or creating it as needed.
    /// `current` is the logged-in user, who gets the provider account linked.
    pub async fn find_or_create_social(&self, social: &SocialUser, current: Option<&User>) -> Result<User, SocialError> {
        if let Some(user) = self.by_social(social.provider, &social.id).await? {
            self.touch_social(social).await?;
            return Ok(user);
        }
        if let Some(user) = current {
            self.link_social(user.id, social).await?;
            return Ok(user.clone());
        }
        let email = social.email.as_deref().ok_or(SocialError::NoEmail)?;
        // The guard: an email only identifies someone when the provider has verified it.
        if social.email_verified != Some(true) && !TRUST_UNVERIFIED_EMAILS {
            return Err(SocialError::UnverifiedEmail);
        }
        let user = match self.by_email(email).await? {
            Some(user) => user,
            None => {
                sqlx::query("insert into users (uuid, name, email, password_hash, email_verified_at) values ($1, $2, $3, null, $4)")
                    .bind(Uuid::<User>::new())
                    .bind(social.name.as_deref())
                    .bind(email)
                    // The provider verified the address (unless TRUST_UNVERIFIED_EMAILS let it through).
                    .bind((social.email_verified == Some(true)).then(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)))
                    .execute(self.db.pool())
                    .await?;
                self.by_email(email).await?.ok_or(sqlx::Error::RowNotFound)?
            }
        };
        self.link_social(user.id, social).await?;
        Ok(user)
    }
}
