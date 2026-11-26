//! OAuth login with third-party providers. GitHub for now.
//!
//! ```ignore
//! // GET /auth/github
//! async fn redirect(cx: Cx<App>, session: Session) -> Redirect {
//!     Redirect::to(cx.github.authorize_url(&session))
//! }
//!
//! // GET /auth/github/callback
//! async fn callback(cx: Cx<App>, (auth, session, Query(cb)): (Auth<User>, Session, Query<Callback>)) -> ... {
//!     let gh = cx.github.user(&session, cb).await?;   // checks `state`
//!     let user = cx.users.upsert_github(gh.id, &gh.login, gh.email.as_deref()).await?;
//!     auth.login(&user);
//!     Ok(Redirect::to_route("dashboard"))
//! }
//! ```
//!
//! Turning a provider user into an app user is app code.

use std::fmt;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use session::Session;

const STATE_KEY: &str = "_oauth_state";

/// The query string the provider redirects back with.
#[derive(Debug, Deserialize)]
pub struct Callback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
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

/// GitHub OAuth app credentials. Build once at startup; cheap to share.
pub struct GitHub {
    client_id: String,
    client_secret: String,
    redirect_url: String,
    scopes: Vec<String>,
    http: reqwest::Client,
    web: String,
    api: String,
}

impl GitHub {
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>, redirect_url: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_url: redirect_url.into(),
            scopes: vec![],
            http: reqwest::Client::new(),
            web: "https://github.com".into(),
            api: "https://api.github.com".into(),
        }
    }

    pub fn scopes(mut self, scopes: &[&str]) -> Self {
        self.scopes = scopes.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Point at another host, for tests or GitHub Enterprise.
    pub fn base_urls(mut self, web: impl Into<String>, api: impl Into<String>) -> Self {
        self.web = web.into();
        self.api = api.into();
        self
    }

    /// Where to send the user. Stores a fresh `state` in the session.
    pub fn authorize_url(&self, session: &Session) -> String {
        let state = random_state();
        session.insert(STATE_KEY, &state).expect("a string serializes");
        let query = serde_urlencoded::to_string([
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", &self.redirect_url),
            ("scope", &self.scopes.join(" ")),
            ("state", &state),
        ])
        .expect("strings urlencode");
        format!("{}/login/oauth/authorize?{query}", self.web)
    }

    /// Finish the login: check `state`, trade the code for a token, fetch the user.
    pub async fn user(&self, session: &Session, cb: Callback) -> Result<GitHubUser, Error> {
        let expected: Option<String> = session.get(STATE_KEY).ok().flatten();
        session.remove(STATE_KEY); // one use, success or not
        match (expected, &cb.state) {
            (Some(e), Some(got)) if ct_eq(&e, got) => {}
            _ => return Err(Error::State),
        }
        if let Some(e) = cb.error {
            return Err(Error::Denied(e));
        }
        let code = cb.code.ok_or_else(|| Error::Denied("no code".into()))?;

        #[derive(Deserialize)]
        struct Token {
            access_token: Option<String>,
            error: Option<String>,
        }
        let token: Token = self
            .http
            .post(format!("{}/login/oauth/access_token", self.web))
            .header("accept", "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", &self.client_secret),
                ("code", &code),
                ("redirect_uri", &self.redirect_url),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let access = match token {
            Token { access_token: Some(t), .. } => t,
            Token { error, .. } => return Err(Error::Denied(error.unwrap_or_else(|| "no access token".into()))),
        };

        let mut user: GitHubUser = self
            .http
            .get(format!("{}/user", self.api))
            .bearer_auth(&access)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "socialite")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        user.token = access;
        Ok(user)
    }
}

fn random_state() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("OS random source");
    URL_SAFE_NO_PAD.encode(b)
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
