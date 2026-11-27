use serde::Deserialize;

use crate::{ClientAuth, Error, OAuth2, Provider, SocialUser, Token};

/// X (Twitter) sign-in with OAuth 2.0 and PKCE. X shares no email address by
/// default, so [`SocialUser::email`] is `None`: link X accounts by id.
pub struct Twitter {
    oauth: OAuth2,
    api: String,
}

/// The user as X reports them.
#[derive(Debug, Clone, Deserialize)]
pub struct TwitterUser {
    pub id: String,
    pub name: String,
    /// The handle, without `@`.
    pub username: String,
    pub profile_image_url: Option<String>,
    #[serde(skip)]
    pub token: String,
    #[serde(skip)]
    pub refresh_token: Option<String>,
}

impl Twitter {
    /// Default scopes: `users.read tweet.read` (X needs both to read the user).
    /// Add `offline.access` for a refresh token.
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>, redirect_url: impl Into<String>) -> Self {
        let mut oauth = OAuth2::new(client_id, client_secret, redirect_url);
        oauth.scopes = ["users.read", "tweet.read"].map(String::from).to_vec();
        oauth.client_auth = ClientAuth::Basic;
        Self { oauth, api: String::new() }.base_urls("https://x.com", "https://api.x.com")
    }

    pub fn scopes(mut self, scopes: &[&str]) -> Self {
        self.oauth.scopes = scopes.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Point at other hosts, for tests.
    pub fn base_urls(mut self, web: impl Into<String>, api: impl Into<String>) -> Self {
        let api = api.into();
        self.oauth.authorize_url = format!("{}/i/oauth2/authorize", web.into());
        self.oauth.token_url = format!("{api}/2/oauth2/token");
        self.api = api;
        self
    }
}

impl Provider for Twitter {
    type User = TwitterUser;

    fn name(&self) -> &'static str {
        "twitter"
    }

    fn oauth(&self) -> &OAuth2 {
        &self.oauth
    }

    async fn fetch_user(&self, token: Token) -> Result<TwitterUser, Error> {
        #[derive(Deserialize)]
        struct Envelope {
            data: TwitterUser,
        }
        let url = format!("{}/2/users/me?user.fields=profile_image_url", self.api);
        let Envelope { data: mut user } = self.oauth.http.get(url).bearer_auth(&token.access_token).send().await?.error_for_status()?.json().await?;
        user.token = token.access_token;
        user.refresh_token = token.refresh_token;
        Ok(user)
    }
}

impl From<TwitterUser> for SocialUser {
    fn from(u: TwitterUser) -> Self {
        SocialUser {
            provider: "twitter",
            id: u.id,
            name: Some(u.name),
            email: None,
            email_verified: None,
            avatar: u.profile_image_url,
            token: u.token,
            refresh_token: u.refresh_token,
        }
    }
}
