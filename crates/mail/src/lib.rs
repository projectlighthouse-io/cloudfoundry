//! Send mail: over SMTP, to the log (development), or into memory (tests).
//!
//! ```ignore
//! let mailer = mail::Mailer::from_url("smtp://user:pass@smtp.example.com:587?tls=required", "Acme <hello@acme.test>")?;
//! mailer.send(mail::Message::new("ada@acme.test", "Verify your email", "Open https://...")).await?;
//!
//! // Tests
//! let mailer = mail::Mailer::memory("hello@acme.test");
//! app_code(&mailer).await;
//! assert_eq!(mailer.sent()[0].subject, "Verify your email");
//! ```
//!
//! The driver is picked by the URL, so environments differ by one setting:
//! `smtp://` (STARTTLS with `?tls=required`, none without), `smtps://`
//! (TLS from the start), `log`, `memory`. Addresses are checked before
//! anything is sent, whatever the driver.
//!
//! SMTP opens one connection per mail (no pool: a pooled transport must be
//! dropped inside a Tokio runtime). Fine for sign-up and reset mail; for bulk
//! mail, send from a queued job.

use std::{
    fmt,
    sync::{Arc, Mutex, PoisonError},
};

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    message::{Mailbox, MultiPart},
};

/// One mail to one address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub to: String,
    pub subject: String,
    /// The plain-text body. Every mail has one.
    pub text: String,
    /// An optional HTML alternative.
    pub html: Option<String>,
}

impl Message {
    pub fn new(to: impl Into<String>, subject: impl Into<String>, text: impl Into<String>) -> Self {
        Self { to: to.into(), subject: subject.into(), text: text.into(), html: None }
    }

    /// Add an HTML part; mail clients show it instead of the text.
    pub fn html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }
}

#[derive(Debug)]
pub enum Error {
    /// A `from` or `to` that is not an email address (`Ada <ada@acme.test>` or `ada@acme.test`).
    Address(String),
    /// A URL no driver takes.
    Url(String),
    /// The SMTP server refused or could not be reached.
    Send(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(a) => write!(f, "`{a}` is not an email address"),
            Self::Url(u) => write!(f, "mail url: {u}"),
            Self::Send(e) => write!(f, "sending mail failed: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Where mail goes. Cheap to clone; clones of a `memory` mailer share what was sent.
#[derive(Clone)]
pub struct Mailer {
    from: String,
    driver: Driver,
}

#[derive(Clone)]
enum Driver {
    Log,
    Memory(Arc<Mutex<Vec<Message>>>),
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
}

fn mailbox(address: &str) -> Result<Mailbox, Error> {
    address.trim().parse().map_err(|_| Error::Address(address.to_owned()))
}

impl Mailer {
    /// Pick the driver from `url`: `smtp://...`, `smtps://...`, `log` or `memory`.
    pub fn from_url(url: &str, from: &str) -> Result<Self, Error> {
        mailbox(from)?;
        let driver = match url.trim() {
            "log" => Driver::Log,
            "memory" => Driver::Memory(Arc::default()),
            u if u.starts_with("smtp://") || u.starts_with("smtps://") => {
                // The URL can hold a password: never echo it in the error.
                let smtp = AsyncSmtpTransport::<Tokio1Executor>::from_url(u).map_err(|e| Error::Url(format!("not a valid SMTP URL ({e})")))?;
                Driver::Smtp(smtp.build())
            }
            _ => return Err(Error::Url("use smtp://..., smtps://..., log or memory".into())),
        };
        Ok(Self { from: from.trim().to_owned(), driver })
    }

    /// Every mail goes to the log, body included: for development only.
    pub fn log(from: &str) -> Self {
        Self { from: from.trim().to_owned(), driver: Driver::Log }
    }

    /// Mail is kept in memory; read it with [`sent`](Self::sent). For tests.
    pub fn memory(from: &str) -> Self {
        Self { from: from.trim().to_owned(), driver: Driver::Memory(Arc::default()) }
    }

    /// The mail a `memory` mailer has sent, oldest first. Empty for other drivers.
    pub fn sent(&self) -> Vec<Message> {
        match &self.driver {
            Driver::Memory(sent) => sent.lock().unwrap_or_else(PoisonError::into_inner).clone(),
            _ => Vec::new(),
        }
    }

    pub async fn send(&self, message: Message) -> Result<(), Error> {
        let to = mailbox(&message.to)?;
        match &self.driver {
            Driver::Log => {
                tracing::info!(to = %to, subject = %message.subject, "mail (log driver; not sent):\n{}", message.text);
                Ok(())
            }
            Driver::Memory(sent) => {
                sent.lock().unwrap_or_else(PoisonError::into_inner).push(message);
                Ok(())
            }
            Driver::Smtp(smtp) => {
                let builder = lettre::Message::builder().from(mailbox(&self.from)?).to(to).subject(message.subject);
                let mail = match message.html {
                    Some(html) => builder.multipart(MultiPart::alternative_plain_html(message.text, html)),
                    None => builder.body(message.text),
                }
                .map_err(|e| Error::Send(e.into()))?;
                smtp.send(mail).await.map(drop).map_err(|e| Error::Send(e.into()))
            }
        }
    }
}

/// `[mail]` settings, with feature `configured`:
///
/// ```toml
/// [mail]
/// url = "secret_env(MAIL_URL, log)"
/// from = "env(MAIL_FROM, Acme <hello@acme.test>)"
/// ```
#[cfg(feature = "configured")]
#[derive(Clone, Debug, configured::serde::Deserialize)]
#[serde(crate = "configured::serde", default, deny_unknown_fields)]
pub struct Config {
    /// `smtp://user:pass@host:587?tls=required`, `smtps://...`, `log` or `memory`. A secret: it may hold a password.
    pub url: configured::SecretString,
    pub from: String,
}

/// `log`, from `hello@example.test`.
#[cfg(feature = "configured")]
impl Default for Config {
    fn default() -> Self {
        Self { url: "log".into(), from: "hello@example.test".into() }
    }
}

#[cfg(feature = "configured")]
impl Mailer {
    pub fn config(cfg: &Config) -> Result<Self, Error> {
        use configured::ExposeSecret;
        Self::from_url(cfg.url.expose_secret(), &cfg.from)
    }
}
