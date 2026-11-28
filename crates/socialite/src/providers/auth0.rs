use crate::{Error, OAuth2, Oidc, OidcUser, Provider, Token};

/// Auth0: OpenID Connect at `https://{domain}`.
pub struct Auth0(Oidc);

impl Auth0 {
    /// From config: credentials, optional scopes, base_url. See [`crate::ProviderConfig`].
    #[cfg(feature = "configured")]
    pub fn from_config(cfg: crate::ProviderConfig) -> Result<Self, crate::ConfigError> {
        let p = cfg.parts(NAME, crate::config::Base::Required)?;
        let base = p.base.unwrap_or_default();
        let s = Self::with_base(&base, p.id, p.secret, p.redirect);
        Ok(match p.scopes {
            Some(scopes) => s.scopes(&scopes.iter().map(String::as_str).collect::<Vec<_>>()),
            None => s,
        })
    }
    /// `domain` is the tenant domain, e.g. `acme.eu.auth0.com` or a custom domain.
    pub fn new(domain: &str, client_id: impl Into<String>, client_secret: impl Into<String>, redirect_url: impl Into<String>) -> Self {
        Self::with_base(&format!("https://{domain}"), client_id, client_secret, redirect_url)
    }

    /// The same, against any base URL, for tests.
    pub fn with_base(base: &str, client_id: impl Into<String>, client_secret: impl Into<String>, redirect_url: impl Into<String>) -> Self {
        Self(Oidc::new("auth0", client_id, client_secret, redirect_url, format!("{base}/authorize"), format!("{base}/oauth/token"), format!("{base}/userinfo")))
    }

    pub fn scopes(self, scopes: &[&str]) -> Self {
        Self(self.0.scopes(scopes))
    }
}

impl Provider for Auth0 {
    type User = OidcUser;

    fn name(&self) -> &'static str {
        "auth0"
    }

    fn oauth(&self) -> &OAuth2 {
        self.0.oauth()
    }

    async fn fetch_user(&self, token: Token) -> Result<OidcUser, Error> {
        self.0.fetch_user(token).await
    }
}

const NAME: &str = "auth0";
