//! Provider credentials from config files (feature `configured`).

use std::fmt;

use configured::{ExposeSecret, SecretString};

/// One provider's credentials. Loaded with `configured`; unknown keys are an
/// error:
///
/// ```toml
/// [socialite.github]
/// client_id = "required_env(GITHUB_CLIENT_ID)"
/// client_secret = "required_secret_env(GITHUB_CLIENT_SECRET)"
/// redirect_url = "https://example.com/auth/github/callback"
/// scopes = ["read:user", "user:email"]   # optional: the provider's defaults
/// ```
///
/// `base_url` is the tenant for Auth0 (`https://acme.eu.auth0.com`), the
/// issuer for OpenID Connect, and a self-hosted instance for GitLab. Other
/// providers reject it: point those elsewhere with their `base_urls` method.
#[derive(Debug, configured::serde::Deserialize)]
#[serde(crate = "configured::serde", deny_unknown_fields)]
pub struct ProviderConfig {
    pub client_id: String,
    pub client_secret: SecretString,
    pub redirect_url: String,
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    #[serde(default)]
    pub base_url: Option<String>,
}

/// A [`ProviderConfig`] that cannot work: empty credentials, a redirect URL
/// that is not absolute `http(s)`, an empty scope, a missing or unsupported
/// `base_url`, or a failed OpenID Connect discovery.
#[derive(Debug)]
pub struct ConfigError(pub(crate) String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "socialite config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

/// How a provider uses `base_url`.
#[derive(Clone, Copy)]
#[allow(dead_code)] // unused when no provider feature is on
pub(crate) enum Base {
    Unsupported,
    Optional,
    Required,
}

#[allow(dead_code)] // unused when no provider feature is on
pub(crate) struct Parts {
    pub id: String,
    pub secret: String,
    pub redirect: String,
    pub scopes: Option<Vec<String>>,
    pub base: Option<String>,
}

fn absolute_http(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    rest.is_some_and(|r| r.split(['/', '?', '#']).next().is_some_and(|host| !host.is_empty()))
}

impl ProviderConfig {
    /// Checks that hold for every provider.
    pub fn check(&self) -> Result<(), ConfigError> {
        let err = |m: &str| Err(ConfigError(m.into()));
        if self.client_id.trim().is_empty() {
            return err("client_id is empty");
        }
        if self.client_secret.expose_secret().trim().is_empty() {
            return err("client_secret is empty");
        }
        if !absolute_http(&self.redirect_url) {
            return err("redirect_url must be an absolute http(s) URL");
        }
        if self.scopes.as_ref().is_some_and(|s| s.iter().any(|s| s.trim().is_empty())) {
            return err("scopes must not contain an empty scope");
        }
        if self.base_url.as_deref().is_some_and(|b| !b.is_empty() && !absolute_http(b)) {
            return err("base_url must be an absolute http(s) URL");
        }
        Ok(())
    }

    #[allow(dead_code)] // unused when no provider feature is on
    pub(crate) fn parts(self, provider: &str, base: Base) -> Result<Parts, ConfigError> {
        self.check()?;
        let url = self.base_url.filter(|b| !b.is_empty()).map(|b| b.trim_end_matches('/').to_owned());
        match (base, &url) {
            (Base::Unsupported, Some(_)) => return Err(ConfigError(format!("{provider} does not take base_url"))),
            (Base::Required, None) => return Err(ConfigError(format!("{provider} needs base_url"))),
            _ => {}
        }
        Ok(Parts {
            id: self.client_id,
            secret: self.client_secret.expose_secret().to_owned(),
            redirect: self.redirect_url,
            scopes: self.scopes,
            base: url,
        })
    }
}
