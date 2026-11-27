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
    /// The public profile email; `None` if the user hides it.
    /// ponytail: add a `/user/emails` lookup (scope `user:email`) when the app needs a private email.
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    /// Access token, for calling the GitHub API on the user's behalf.
    #[serde(skip)]
    pub token: String,
}

impl GitHub {
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
    const NAME: &'static str = "github";

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
        user.token = token.access_token;
        Ok(user)
    }
}

impl From<GitHubUser> for SocialUser {
    fn from(u: GitHubUser) -> Self {
        SocialUser {
            provider: GitHub::NAME,
            id: u.id.to_string(),
            name: u.name.or(Some(u.login)),
            email: u.email,
            // GitHub's public profile email carries no verified flag.
            email_verified: None,
            avatar: u.avatar_url,
            token: u.token,
            refresh_token: None,
        }
    }
}
