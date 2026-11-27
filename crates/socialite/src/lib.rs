//! OAuth login with third-party providers.
//!
//! Each provider ([`GitHub`], [`Google`]) implements [`Provider`]: its
//! endpoints and how to fetch the user. The flow is shared:
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
//!     let social: SocialUser = cx.google.user(&session, cb).await?.into();   // checks `state`, trades the code
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

mod providers;
pub use providers::{GitHub, GitHubUser, Google, GoogleUser};

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
    /// [`Provider::NAME`], e.g. `"google"`.
    pub provider: &'static str,
    /// The provider's stable id for this user.
    pub id: String,
    pub name: Option<String>,
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

/// Client credentials and endpoints: the part every provider shares.
pub struct OAuth2 {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
    pub scopes: Vec<String>,
    pub authorize_url: String,
    pub token_url: String,
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
            http: reqwest::Client::new(),
        }
    }
}

/// One OAuth 2.0 provider. Implement the required items; the flow
/// ([`authorize_url`](Self::authorize_url), [`user`](Self::user)) is provided.
pub trait Provider: Send + Sync {
    /// The provider's user, as its API returns it.
    type User: Into<SocialUser> + Send;

    /// Short name, used in session keys and in [`SocialUser::provider`].
    const NAME: &'static str;

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
        session.insert(&state_key::<Self>(), &state).expect("a string serializes");
        session.insert(&pkce_key::<Self>(), &verifier).expect("a string serializes");
        let o = self.oauth();
        let mut params = vec![
            ("client_id", o.client_id.clone()),
            ("response_type", "code".to_owned()),
            ("redirect_uri", o.redirect_url.clone()),
            ("scope", o.scopes.join(" ")),
            ("code_challenge", URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))),
            ("code_challenge_method", "S256".to_owned()),
        ];
        params.extend(self.extra_params());
        params.push(("state", state));
        format!("{}?{}", o.authorize_url, serde_urlencoded::to_string(params).expect("strings urlencode"))
    }

    /// Finish the login: check `state`, trade the code (with the PKCE
    /// verifier) for a token, fetch the user.
    fn user(&self, session: &Session, cb: Callback) -> impl Future<Output = Result<Self::User, Error>> + Send {
        async move {
            let expected: Option<String> = session.get(&state_key::<Self>()).ok().flatten();
            let verifier: Option<String> = session.get(&pkce_key::<Self>()).ok().flatten();
            session.remove(&state_key::<Self>()); // one use, success or not
            session.remove(&pkce_key::<Self>());
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
}

async fn exchange(o: &OAuth2, code: &str, verifier: Option<&str>) -> Result<Token, Error> {
    #[derive(Deserialize)]
    struct Raw {
        access_token: Option<String>,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
        error: Option<String>,
    }
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("client_id", o.client_id.as_str()),
        ("client_secret", &o.client_secret),
        ("code", code),
        ("redirect_uri", &o.redirect_url),
    ];
    if let Some(v) = verifier {
        form.push(("code_verifier", v));
    }
    let raw: Raw = o.http.post(&o.token_url).header("accept", "application/json").form(&form).send().await?.error_for_status()?.json().await?;
    match raw {
        Raw { access_token: Some(access_token), refresh_token, expires_in, .. } => Ok(Token { access_token, refresh_token, expires_in }),
        Raw { error, .. } => Err(Error::Denied(error.unwrap_or_else(|| "no access token".into()))),
    }
}

fn state_key<P: Provider + ?Sized>() -> String {
    format!("_oauth_state:{}", P::NAME)
}

fn pkce_key<P: Provider + ?Sized>() -> String {
    format!("_oauth_pkce:{}", P::NAME)
}

fn random() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("OS random source");
    URL_SAFE_NO_PAD.encode(b)
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
