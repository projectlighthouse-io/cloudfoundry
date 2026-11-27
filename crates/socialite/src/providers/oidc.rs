use serde::Deserialize;

use crate::{Error, OAuth2, Provider, SocialUser, Token};

/// Any OpenID Connect provider (Okta, Keycloak, Azure AD, Authentik, ...):
/// endpoints from the issuer's discovery document, or set explicitly.
pub struct Oidc {
    name: &'static str,
    oauth: OAuth2,
    userinfo_url: String,
}

/// The standard OpenID Connect claims, plus every claim as JSON.
#[derive(Debug, Clone, Deserialize)]
pub struct OidcUser {
    pub sub: String,
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: bool,
    pub name: Option<String>,
    pub preferred_username: Option<String>,
    pub picture: Option<String>,
    /// All claims the provider returned, for custom ones (groups, roles, ...).
    #[serde(skip)]
    pub claims: serde_json::Value,
    #[serde(skip)]
    pub token: String,
    #[serde(skip)]
    pub refresh_token: Option<String>,
}

impl Oidc {
    /// Explicit endpoints. `name` is the provider's short name in routes and
    /// session keys, e.g. `"okta"`. Default scopes: `openid email profile`.
    pub fn new(
        name: &'static str,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_url: impl Into<String>,
        authorize_url: impl Into<String>,
        token_url: impl Into<String>,
        userinfo_url: impl Into<String>,
    ) -> Self {
        let mut oauth = OAuth2::new(client_id, client_secret, redirect_url);
        oauth.scopes = ["openid", "email", "profile"].map(String::from).to_vec();
        oauth.authorize_url = authorize_url.into();
        oauth.token_url = token_url.into();
        Self { name, oauth, userinfo_url: userinfo_url.into() }
    }

    /// Endpoints from `{issuer}/.well-known/openid-configuration`. Call once
    /// at startup.
    pub async fn discover(
        name: &'static str,
        issuer: &str,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_url: impl Into<String>,
    ) -> Result<Self, Error> {
        #[derive(Deserialize)]
        struct Discovery {
            authorization_endpoint: String,
            token_endpoint: String,
            userinfo_endpoint: String,
        }
        let url = format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'));
        let d: Discovery = reqwest::get(url).await?.error_for_status()?.json().await?;
        Ok(Self::new(name, client_id, client_secret, redirect_url, d.authorization_endpoint, d.token_endpoint, d.userinfo_endpoint))
    }

    pub fn scopes(mut self, scopes: &[&str]) -> Self {
        self.oauth.scopes = scopes.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Turn PKCE off, for the rare provider that rejects it.
    pub fn without_pkce(mut self) -> Self {
        self.oauth.pkce = false;
        self
    }

    /// Authenticate to the token endpoint with HTTP Basic instead of the form.
    pub fn basic_auth(mut self) -> Self {
        self.oauth.client_auth = crate::ClientAuth::Basic;
        self
    }
}

impl Provider for Oidc {
    type User = OidcUser;

    fn name(&self) -> &'static str {
        self.name
    }

    fn oauth(&self) -> &OAuth2 {
        &self.oauth
    }

    async fn fetch_user(&self, token: Token) -> Result<OidcUser, Error> {
        let claims: serde_json::Value =
            self.oauth.http.get(&self.userinfo_url).bearer_auth(&token.access_token).send().await?.error_for_status()?.json().await?;
        let mut user: OidcUser = serde_json::from_value(claims.clone()).map_err(|e| Error::Denied(format!("userinfo: {e}")))?;
        user.claims = claims;
        user.token = token.access_token;
        user.refresh_token = token.refresh_token;
        Ok(user)
    }
}

impl From<OidcUser> for SocialUser {
    /// `provider` is set by [`Provider::social_user`] to the provider's own name.
    fn from(u: OidcUser) -> Self {
        SocialUser {
            provider: "oidc",
            id: u.sub,
            name: u.name.or(u.preferred_username),
            email: u.email,
            email_verified: Some(u.email_verified),
            avatar: u.picture,
            token: u.token,
            refresh_token: u.refresh_token,
        }
    }
}
