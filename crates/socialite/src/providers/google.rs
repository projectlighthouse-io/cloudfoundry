use serde::Deserialize;

use crate::{Error, OAuth2, Provider, SocialUser, Token};

/// Google sign-in (OpenID Connect). Build once at startup; cheap to share.
///
/// The user comes from the userinfo endpoint, called with the token from the
/// direct code exchange, so no ID token needs verifying.
pub struct Google {
    oauth: OAuth2,
    userinfo_url: String,
    hosted_domain: Option<String>,
    offline: bool,
    prompt: Option<String>,
}

/// The user as Google reports them.
#[derive(Debug, Clone, Deserialize)]
pub struct GoogleUser {
    /// Google's stable id for the account. Emails can change; this does not.
    pub sub: String,
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: bool,
    pub name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub picture: Option<String>,
    /// The Google Workspace domain, for Workspace accounts.
    pub hd: Option<String>,
    #[serde(skip)]
    pub token: String,
    /// Present with [`Google::offline`] on the first consent.
    #[serde(skip)]
    pub refresh_token: Option<String>,
}

impl Google {
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
    /// Default scopes: `openid email profile`.
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>, redirect_url: impl Into<String>) -> Self {
        let mut oauth = OAuth2::new(client_id, client_secret, redirect_url);
        oauth.scopes = ["openid", "email", "profile"].map(String::from).to_vec();
        Self { oauth, userinfo_url: String::new(), hosted_domain: None, offline: false, prompt: None }.base_urls(
            "https://accounts.google.com/o/oauth2/v2/auth",
            "https://oauth2.googleapis.com/token",
            "https://openidconnect.googleapis.com/v1/userinfo",
        )
    }

    pub fn scopes(mut self, scopes: &[&str]) -> Self {
        self.oauth.scopes = scopes.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Only accounts of this Google Workspace domain. Sent to Google as a
    /// hint, and checked against the returned user, since the hint alone can
    /// be removed from the URL.
    pub fn hosted_domain(mut self, domain: impl Into<String>) -> Self {
        self.hosted_domain = Some(domain.into());
        self
    }

    /// Ask for a refresh token, to call Google APIs while the user is away.
    pub fn offline(mut self) -> Self {
        self.offline = true;
        self
    }

    /// `select_account`, `consent` or `none`.
    pub fn prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    /// Point at other endpoints, for tests.
    pub fn base_urls(mut self, authorize: impl Into<String>, token: impl Into<String>, userinfo: impl Into<String>) -> Self {
        self.oauth.authorize_url = authorize.into();
        self.oauth.token_url = token.into();
        self.userinfo_url = userinfo.into();
        self
    }
}

impl Provider for Google {
    type User = GoogleUser;
    fn name(&self) -> &'static str {
        NAME
    }

    fn oauth(&self) -> &OAuth2 {
        &self.oauth
    }

    fn extra_params(&self) -> Vec<(&'static str, String)> {
        let mut p = Vec::new();
        if let Some(d) = &self.hosted_domain {
            p.push(("hd", d.clone()));
        }
        if self.offline {
            p.push(("access_type", "offline".to_owned()));
        }
        if let Some(prompt) = &self.prompt {
            p.push(("prompt", prompt.clone()));
        }
        p
    }

    async fn fetch_user(&self, token: Token) -> Result<GoogleUser, Error> {
        let mut user: GoogleUser =
            self.oauth.http.get(&self.userinfo_url).bearer_auth(&token.access_token).send().await?.error_for_status()?.json().await?;
        if let Some(want) = &self.hosted_domain {
            if user.hd.as_deref() != Some(want.as_str()) {
                return Err(Error::Denied(format!("account is not in the {want} domain")));
            }
        }
        user.token = token.access_token;
        user.refresh_token = token.refresh_token;
        Ok(user)
    }
}

impl From<GoogleUser> for SocialUser {
    fn from(u: GoogleUser) -> Self {
        SocialUser {
            provider: "google",
            id: u.sub,
            name: u.name,
            username: None,
            email: u.email,
            email_verified: Some(u.email_verified),
            avatar: u.picture,
            token: u.token,
            refresh_token: u.refresh_token,
        }
    }
}

const NAME: &str = "google";
