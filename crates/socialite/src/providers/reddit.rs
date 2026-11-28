use serde::Deserialize;

use crate::{ClientAuth, Error, OAuth2, Provider, SocialUser, Token};

/// Reddit sign-in. Reddit shares no email address, so [`SocialUser::email`]
/// is always `None`: link Reddit accounts by id.
pub struct Reddit {
    oauth: OAuth2,
    api: String,
    permanent: bool,
}

/// The user as Reddit reports them.
#[derive(Debug, Clone, Deserialize)]
pub struct RedditUser {
    pub id: String,
    /// The username, without `u/`.
    pub name: String,
    pub icon_img: Option<String>,
    #[serde(skip)]
    pub token: String,
    #[serde(skip)]
    pub refresh_token: Option<String>,
}

impl Reddit {
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
    /// Default scope: `identity`. Reddit asks API clients for a descriptive
    /// User-Agent: set one with [`Reddit::user_agent`].
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>, redirect_url: impl Into<String>) -> Self {
        let mut oauth = OAuth2::new(client_id, client_secret, redirect_url);
        oauth.scopes = vec!["identity".into()];
        oauth.client_auth = ClientAuth::Basic;
        oauth.pkce = false; // not supported by Reddit
        Self { oauth, api: String::new(), permanent: false }.base_urls("https://www.reddit.com", "https://oauth.reddit.com")
    }

    pub fn scopes(mut self, scopes: &[&str]) -> Self {
        self.oauth.scopes = scopes.iter().map(|s| s.to_string()).collect();
        self
    }

    /// e.g. `"web:myapp:v1.0 (by /u/me)"`.
    pub fn user_agent(mut self, ua: &str) -> Self {
        if let Ok(client) = reqwest::Client::builder().user_agent(ua).build() {
            self.oauth.http = client;
        }
        self
    }

    /// Ask for a refresh token (`duration=permanent`).
    pub fn permanent(mut self) -> Self {
        self.permanent = true;
        self
    }

    /// Point at other hosts, for tests.
    pub fn base_urls(mut self, web: impl Into<String>, api: impl Into<String>) -> Self {
        let web = web.into();
        self.oauth.authorize_url = format!("{web}/api/v1/authorize");
        self.oauth.token_url = format!("{web}/api/v1/access_token");
        self.api = api.into();
        self
    }
}

impl Provider for Reddit {
    type User = RedditUser;

    fn name(&self) -> &'static str {
        NAME
    }

    fn oauth(&self) -> &OAuth2 {
        &self.oauth
    }

    fn extra_params(&self) -> Vec<(&'static str, String)> {
        vec![("duration", if self.permanent { "permanent" } else { "temporary" }.to_owned())]
    }

    async fn fetch_user(&self, token: Token) -> Result<RedditUser, Error> {
        let mut user: RedditUser =
            self.oauth.http.get(format!("{}/api/v1/me", self.api)).bearer_auth(&token.access_token).send().await?.error_for_status()?.json().await?;
        // Reddit escapes `&` in icon URLs as `&amp;`.
        user.icon_img = user.icon_img.map(|u| u.replace("&amp;", "&")).filter(|u| !u.is_empty());
        user.token = token.access_token;
        user.refresh_token = token.refresh_token;
        Ok(user)
    }
}

impl From<RedditUser> for SocialUser {
    fn from(u: RedditUser) -> Self {
        SocialUser {
            provider: "reddit",
            id: u.id,
            name: Some(u.name.clone()),
            username: Some(u.name),
            email: None,
            email_verified: None,
            avatar: u.icon_img,
            token: u.token,
            refresh_token: u.refresh_token,
        }
    }
}

const NAME: &str = "reddit";
