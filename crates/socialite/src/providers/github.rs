use serde::Deserialize;

use crate::{Error, OAuth2, Provider, SocialUser, Token};

/// GitHub OAuth app credentials. Build once at startup; cheap to share.
pub struct GitHub {
    oauth: OAuth2,
    api: String,
}

/// The user as GitHub reports them.
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubUser {
    pub id: u64,
    pub login: String,
    pub name: Option<String>,
    /// The verified primary email when the `user:email` scope was granted,
    /// otherwise the public profile email (`None` if the user hides it).
    pub email: Option<String>,
    /// `Some(true)` for the verified primary email, `Some(false)` when GitHub
    /// lists no verified primary email, `None` without the `user:email` scope.
    #[serde(skip)]
    pub email_verified: Option<bool>,
    pub avatar_url: Option<String>,
    /// Access token, for calling the GitHub API on the user's behalf.
    #[serde(skip)]
    pub token: String,
}

impl GitHub {
    /// From config: credentials, optional scopes. See [`crate::ProviderConfig`].
    #[cfg(feature = "configured")]
    pub fn from_config(cfg: crate::ProviderConfig) -> Result<Self, crate::ConfigError> {
        let p = cfg.parts(NAME, crate::config::Base::Unsupported)?;
        let mut s = Self::new(p.id, p.secret, p.redirect);
        if let Some(scopes) = p.scopes {
            s.oauth.scopes = scopes;
        }
        Ok(s)
    }
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>, redirect_url: impl Into<String>) -> Self {
        Self { oauth: OAuth2::new(client_id, client_secret, redirect_url), api: String::new() }.base_urls("https://github.com", "https://api.github.com")
    }

    pub fn scopes(mut self, scopes: &[&str]) -> Self {
        self.oauth.scopes = scopes.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Point at another host, for tests or GitHub Enterprise.
    pub fn base_urls(mut self, web: impl Into<String>, api: impl Into<String>) -> Self {
        let web = web.into();
        self.oauth.authorize_url = format!("{web}/login/oauth/authorize");
        self.oauth.token_url = format!("{web}/login/oauth/access_token");
        self.api = api.into();
        self
    }
}

impl Provider for GitHub {
    type User = GitHubUser;
    fn name(&self) -> &'static str {
        NAME
    }

    fn oauth(&self) -> &OAuth2 {
        &self.oauth
    }

    async fn fetch_user(&self, token: Token) -> Result<GitHubUser, Error> {
        let mut user: GitHubUser = self
            .oauth
            .http
            .get(format!("{}/user", self.api))
            .bearer_auth(&token.access_token)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "socialite")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        #[derive(Deserialize)]
        struct Email {
            email: String,
            primary: bool,
            verified: bool,
        }
        // Needs the `user:email` scope; without it GitHub answers 403/404 and we
        // keep the profile email, unverified-unknown.
        let emails = self
            .oauth
            .http
            .get(format!("{}/user/emails", self.api))
            .bearer_auth(&token.access_token)
            .header("accept", "application/vnd.github+json")
            .send()
            .await?;
        if emails.status().is_success() {
            let emails: Vec<Email> = emails.json().await?;
            match emails.into_iter().find(|e| e.primary && e.verified) {
                Some(e) => (user.email, user.email_verified) = (Some(e.email), Some(true)),
                None => user.email_verified = Some(false),
            }
        }
        user.token = token.access_token;
        Ok(user)
    }
}

impl From<GitHubUser> for SocialUser {
    fn from(u: GitHubUser) -> Self {
        SocialUser {
            provider: "github",
            id: u.id.to_string(),
            name: u.name.or_else(|| Some(u.login.clone())),
            username: Some(u.login),
            email: u.email,
            email_verified: u.email_verified,
            avatar: u.avatar_url,
            token: u.token,
            refresh_token: None,
        }
    }
}

const NAME: &str = "github";
