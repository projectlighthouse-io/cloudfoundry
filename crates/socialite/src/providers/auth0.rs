use crate::{Error, OAuth2, Oidc, OidcUser, Provider, Token};

/// Auth0: OpenID Connect at `https://{domain}`.
pub struct Auth0(Oidc);

impl Auth0 {
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
