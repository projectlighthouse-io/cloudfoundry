//! OAuth login with third-party providers.
//!
//! Each provider implements [`Provider`]: its endpoints and how to fetch the
//! user. The flow is shared. Providers are Cargo features, so an app compiles
//! only the ones it uses:
//!
//! | Feature | Provider |
//! | --- | --- |
//! | `github` | [`GitHub`] |
//! | `gitlab` | `GitLab` (gitlab.com or self-hosted) |
//! | `google` | `Google` |
//! | `reddit` | `Reddit` |
//! | `twitter` | `Twitter` (X) |
//! | `oidc` | `Oidc`: any OpenID Connect provider, by discovery or explicit endpoints |
//! | `auth0` | `Auth0` (implies `oidc`) |
//!
//! Feature `route` adds [`routes`]: `GET /auth/{name}` and its callback.
//!
//! ```ignore
//! use socialite::Provider;
//!
//! // GET /auth/google
//! async fn redirect(cx: Cx<App>, session: Session) -> Redirect {
//!     Redirect::to(cx.google.authorize_url(&session))      // stores `state` and a PKCE verifier
//! }
//!
//! // GET /auth/google/callback
//! async fn callback(cx: Cx<App>, (auth, session, Query(cb)): (Auth<User>, Session, Query<Callback>)) -> ... {
//!     let social = cx.google.social_user(&session, cb).await?;   // checks `state`, trades the code
//!     let user = cx.users.find_or_create_social(&social).await?;
//!     auth.login(&user);
//!     Ok(Redirect::to_route("dashboard"))
//! }
//! ```
//!
//! Turning a provider user into an app user is app code (`art publish socialite`).

use std::{fmt, future::Future};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use session::Session;
use sha2::{Digest, Sha256};

#[cfg(feature = "configured")]
mod config;
mod providers;
#[cfg(feature = "configured")]
pub use config::{ConfigError, ProviderConfig};
#[cfg(feature = "route")]
mod route;
#[cfg(feature = "route")]
pub use route::{SocialCallback, routes};
#[allow(unused_imports)] // empty when no provider feature is on
pub use providers::*;

/// The query string the provider redirects back with.
#[derive(Debug, Deserialize)]
pub struct Callback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// What the token endpoint returns.
#[derive(Debug, Clone, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Seconds until `access_token` expires, when the provider says.
    pub expires_in: Option<u64>,
}

/// Any provider's user, in one shape: for app code that handles every
/// provider the same way (find or create the account, log in).
#[derive(Debug, Clone)]
pub struct SocialUser {
    /// [`Provider::name`], e.g. `"google"`.
    pub provider: &'static str,
    /// The provider's stable id for this user.
    pub id: String,
    pub name: Option<String>,
    /// The handle on the provider (GitHub login, X username, ...); `None` when it has none (Google).
    pub username: Option<String>,
    pub email: Option<String>,
    /// Whether the provider vouches for `email`; `None` when it does not say.
    pub email_verified: Option<bool>,
    pub avatar: Option<String>,
    pub token: String,
    pub refresh_token: Option<String>,
}

#[derive(Debug)]
pub enum Error {
    /// `state` missing or not the one this session issued: a forged or replayed callback.
    State,
    /// The user declined, or the provider reported an error.
    Denied(String),
    /// Transport or decoding failure talking to the provider.
    Http(reqwest::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::State => f.write_str("oauth state mismatch"),
            Error::Denied(e) => write!(f, "oauth denied: {e}"),
            Error::Http(e) => write!(f, "oauth request failed: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e)
    }
}

/// How the client authenticates to the token endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientAuth {
    /// `client_secret` in the form body (`client_secret_post`).
    Post,
    /// HTTP Basic with the id and secret (`client_secret_basic`), as Reddit and X require.
    Basic,
}

/// Client credentials and endpoints: the part every provider shares.
pub struct OAuth2 {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
    pub scopes: Vec<String>,
    pub authorize_url: String,
    pub token_url: String,
    pub client_auth: ClientAuth,
    /// Send PKCE (`S256`). On by default; off for providers that reject it.
    pub pkce: bool,
    pub http: reqwest::Client,
}

impl OAuth2 {
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>, redirect_url: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_url: redirect_url.into(),
            scopes: Vec::new(),
            authorize_url: String::new(),
            token_url: String::new(),
            client_auth: ClientAuth::Post,
            pkce: true,
            http: reqwest::Client::builder().user_agent("socialite").build().unwrap_or_default(),
        }
    }
}

/// One OAuth 2.0 provider. Implement the required items; the flow
/// ([`authorize_url`](Self::authorize_url), [`user`](Self::user)) is provided.
pub trait Provider: Send + Sync {
    /// The provider's user, as its API returns it.
    type User: Into<SocialUser> + Send;

    /// Short name, used in session keys, routes and [`SocialUser::provider`].
    fn name(&self) -> &'static str;

    fn oauth(&self) -> &OAuth2;

    /// Extra authorize parameters (`prompt`, `hd`, `access_type`, ...).
    fn extra_params(&self) -> Vec<(&'static str, String)> {
        Vec::new()
    }

    /// Fetch the user with a fresh token, and check anything provider-specific.
    fn fetch_user(&self, token: Token) -> impl Future<Output = Result<Self::User, Error>> + Send;

    /// Where to send the user. Stores a fresh `state` and PKCE verifier in the session.
    fn authorize_url(&self, session: &Session) -> String {
        let (state, verifier) = (random(), random());
        let o = self.oauth();
        // Strings always serialize, so these inserts cannot fail.
        let _ = session.insert(&key("state", self.name()), &state);
        let mut params = vec![
            ("client_id", o.client_id.clone()),
            ("response_type", "code".to_owned()),
            ("redirect_uri", o.redirect_url.clone()),
            ("scope", o.scopes.join(" ")),
        ];
        if o.pkce {
            let _ = session.insert(&key("pkce", self.name()), &verifier);
            params.push(("code_challenge", URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))));
            params.push(("code_challenge_method", "S256".to_owned()));
        }
        params.extend(self.extra_params());
        params.push(("state", state));
        // Pairs of strings always encode.
        format!("{}?{}", o.authorize_url, serde_urlencoded::to_string(params).unwrap_or_default())
    }

    /// Finish the login: check `state`, trade the code (with the PKCE
    /// verifier) for a token, fetch the user.
    fn user(&self, session: &Session, cb: Callback) -> impl Future<Output = Result<Self::User, Error>> + Send {
        async move {
            let (state_key, pkce_key) = (key("state", self.name()), key("pkce", self.name()));
            let expected: Option<String> = session.get(&state_key).ok().flatten();
            let verifier: Option<String> = session.get(&pkce_key).ok().flatten();
            session.remove(&state_key); // one use, success or not
            session.remove(&pkce_key);
            match (expected, &cb.state) {
                (Some(e), Some(got)) if ct_eq(&e, got) => {}
                _ => return Err(Error::State),
            }
            if let Some(e) = cb.error {
                return Err(Error::Denied(e));
            }
            let code = cb.code.ok_or_else(|| Error::Denied("no code".into()))?;
            let token = exchange(self.oauth(), &code, verifier.as_deref()).await?;
            self.fetch_user(token).await
        }
    }

    /// [`user`](Self::user) as a [`SocialUser`] tagged with this provider's
    /// [`name`](Self::name): one code path for every provider.
    fn social_user(&self, session: &Session, cb: Callback) -> impl Future<Output = Result<SocialUser, Error>> + Send {
        async move {
            let mut social: SocialUser = self.user(session, cb).await?.into();
            social.provider = self.name();
            Ok(social)
        }
    }
}

async fn exchange(o: &OAuth2, code: &str, verifier: Option<&str>) -> Result<Token, Error> {
    #[derive(Deserialize)]
    struct Raw {
        access_token: Option<String>,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
        error: Option<String>,
    }
    let mut form = vec![("grant_type", "authorization_code"), ("client_id", o.client_id.as_str()), ("code", code), ("redirect_uri", &o.redirect_url)];
    if let Some(v) = verifier {
        form.push(("code_verifier", v));
    }
    let mut req = o.http.post(&o.token_url).header("accept", "application/json");
    match o.client_auth {
        ClientAuth::Post => form.push(("client_secret", &o.client_secret)),
        ClientAuth::Basic => req = req.basic_auth(&o.client_id, Some(&o.client_secret)),
    }
    let raw: Raw = req.form(&form).send().await?.error_for_status()?.json().await?;
    match raw {
        Raw { access_token: Some(access_token), refresh_token, expires_in, .. } => Ok(Token { access_token, refresh_token, expires_in }),
        Raw { error, .. } => Err(Error::Denied(error.unwrap_or_else(|| "no access token".into()))),
    }
}

fn key(kind: &str, provider: &str) -> String {
    format!("_oauth_{kind}:{provider}")
}

fn random() -> String {
    let mut b = [0u8; 32];
    // No secure fallback exists: a guessable state or PKCE verifier must not be issued.
    #[allow(clippy::expect_used, reason = "the OS random source failed; there is no safe fallback")]
    getrandom::fill(&mut b).expect("OS random source");
    URL_SAFE_NO_PAD.encode(b)
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
