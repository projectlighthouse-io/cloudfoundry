use serde::Deserialize;

use crate::{Error, OAuth2, Provider, SocialUser, Token};

/// GitLab sign-in, on gitlab.com or a self-hosted instance ([`GitLab::base_url`]).
/// Uses GitLab's OpenID Connect userinfo, which says whether the email is verified.
pub struct GitLab {
    oauth: OAuth2,
    userinfo_url: String,
}

/// The user as GitLab's userinfo reports them.
#[derive(Debug, Clone, Deserialize)]
pub struct GitLabUser {
    /// GitLab's user id, as a string.
    pub sub: String,
    /// The username.
    pub nickname: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: bool,
    pub picture: Option<String>,
    #[serde(skip)]
    pub token: String,
    #[serde(skip)]
    pub refresh_token: Option<String>,
}

impl GitLab {
    /// From config: credentials, optional scopes, base_url. See [`crate::ProviderConfig`].
    #[cfg(feature = "configured")]
    pub fn from_config(cfg: crate::ProviderConfig) -> Result<Self, crate::ConfigError> {
        let p = cfg.parts(NAME, crate::config::Base::Optional)?;
        let mut s = Self::new(p.id, p.secret, p.redirect);
        if let Some(base) = p.base {
            s = s.base_url(base);
        }
        if let Some(scopes) = p.scopes {
            s.oauth.scopes = scopes;
        }
        Ok(s)
    }
    /// Default scopes: `openid email profile`.
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>, redirect_url: impl Into<String>) -> Self {
        let mut oauth = OAuth2::new(client_id, client_secret, redirect_url);
        oauth.scopes = ["openid", "email", "profile"].map(String::from).to_vec();
        Self { oauth, userinfo_url: String::new() }.base_url("https://gitlab.com")
    }

    pub fn scopes(mut self, scopes: &[&str]) -> Self {
        self.oauth.scopes = scopes.iter().map(|s| s.to_string()).collect();
        self
    }

    /// A self-hosted GitLab, e.g. `https://gitlab.example.com`.
    pub fn base_url(mut self, base: impl Into<String>) -> Self {
        let base = base.into();
        self.oauth.authorize_url = format!("{base}/oauth/authorize");
        self.oauth.token_url = format!("{base}/oauth/token");
        self.userinfo_url = format!("{base}/oauth/userinfo");
        self
    }
}

impl Provider for GitLab {
    type User = GitLabUser;

    fn name(&self) -> &'static str {
        "gitlab"
    }

    fn oauth(&self) -> &OAuth2 {
        &self.oauth
    }

    async fn fetch_user(&self, token: Token) -> Result<GitLabUser, Error> {
        let mut user: GitLabUser =
            self.oauth.http.get(&self.userinfo_url).bearer_auth(&token.access_token).send().await?.error_for_status()?.json().await?;
        user.token = token.access_token;
        user.refresh_token = token.refresh_token;
        Ok(user)
    }
}

impl From<GitLabUser> for SocialUser {
    fn from(u: GitLabUser) -> Self {
        SocialUser {
            provider: "gitlab",
            id: u.sub,
            name: u.name.or_else(|| u.nickname.clone()),
            username: u.nickname,
            email: u.email,
            email_verified: Some(u.email_verified),
            avatar: u.picture,
            token: u.token,
            refresh_token: u.refresh_token,
        }
    }
}

const NAME: &str = "gitlab";
