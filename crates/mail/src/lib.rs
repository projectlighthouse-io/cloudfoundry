//! Send mail: over SMTP, to the log (development), or into memory (tests),
//! from Markdown-ish templates, now or through the `queue` crate.
//!
//! ```ignore
//! let mailer = mail::from_url("smtp://user:pass@smtp.example.com:587?tls=required", "Acme <hello@acme.test>")?;
//! let template = mail::Template::parse(include_str!("../resources/mail/verify-email.md"))?;
//! mailer.send(template.message("ada@acme.test", &[("link", &link)])?).await?;
//!
//! // From a worker instead (feature `queue`):
//! mail::send_later::<App, _>(&app.queue, message).await?;
//!
//! // Tests
//! let mailer = mail::MemoryMailer::new("hello@acme.test");
//! app_code(&mailer).await;
//! assert_eq!(mailer.sent()[0].subject, "Verify your email address");
//! ```
//!
//! [`from_url`] picks the [`Mailer`] by URL, so environments differ by one
//! setting: `smtp://` (STARTTLS with `?tls=required`, none without),
//! `smtps://` (TLS from the start), `log`, `memory`. Addresses are checked
//! before anything is sent, whatever the mailer.
//!
//! SMTP opens one connection per mail (no pool: a pooled transport must be
//! dropped inside a Tokio runtime). Fine for sign-up and reset mail; send
//! bulk mail through the queue.

use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
};

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    message::{Mailbox, MultiPart},
};
use serde::{Deserialize, Serialize};

mod template;
pub use template::Template;

#[cfg(feature = "queue")]
mod queued;
#[cfg(feature = "queue")]
pub use queued::{HasMailer, SendMail, send_later};

/// One mail to one address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// A URL no mailer takes.
    Url(String),
    /// A template without a `Subject:` line, or a `{{name}}` with no value.
    Template(String),
    /// The SMTP server refused or could not be reached.
    Send(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(a) => write!(f, "`{a}` is not an email address"),
            Self::Url(u) => write!(f, "mail url: {u}"),
            Self::Template(t) => write!(f, "mail template: {t}"),
            Self::Send(e) => write!(f, "sending mail failed: {e}"),
        }
    }
}

impl std::error::Error for Error {}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Where mail goes. Object-safe, so the app holds a `Box<dyn Mailer>` chosen
/// by config ([`from_url`]); implement it to send through another service.
pub trait Mailer: Send + Sync + 'static {
    /// The sender, `Acme <hello@acme.test>`.
    fn from(&self) -> &str;
    fn send(&self, message: Message) -> BoxFuture<'_, Result<(), Error>>;
}

fn mailbox(address: &str) -> Result<Mailbox, Error> {
    address.trim().parse().map_err(|_| Error::Address(address.to_owned()))
}

/// Pick the mailer from `url`: `smtp://...`, `smtps://...`, `log` or `memory`.
pub fn from_url(url: &str, from: &str) -> Result<Box<dyn Mailer>, Error> {
    mailbox(from)?;
    Ok(match url.trim() {
        "log" => Box::new(LogMailer::new(from)),
        "memory" => Box::new(MemoryMailer::new(from)),
        u if u.starts_with("smtp://") || u.starts_with("smtps://") => Box::new(SmtpMailer::new(u, from)?),
        _ => return Err(Error::Url("use smtp://..., smtps://..., log or memory".into())),
    })
}

/// SMTP through lettre.
#[derive(Clone)]
pub struct SmtpMailer {
    from: String,
    smtp: AsyncSmtpTransport<Tokio1Executor>,
}

impl SmtpMailer {
    pub fn new(url: &str, from: &str) -> Result<Self, Error> {
        mailbox(from)?;
        // The URL can hold a password: never echo it in the error.
        let smtp = AsyncSmtpTransport::<Tokio1Executor>::from_url(url).map_err(|e| Error::Url(format!("not a valid SMTP URL ({e})")))?;
        Ok(Self { from: from.trim().to_owned(), smtp: smtp.build() })
    }
}

impl Mailer for SmtpMailer {
    fn from(&self) -> &str {
        &self.from
    }

    fn send(&self, message: Message) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let builder = lettre::Message::builder().from(mailbox(&self.from)?).to(mailbox(&message.to)?).subject(message.subject);
            let mail = match message.html {
                Some(html) => builder.multipart(MultiPart::alternative_plain_html(message.text, html)),
                None => builder.body(message.text),
            }
            .map_err(|e| Error::Send(e.into()))?;
            self.smtp.send(mail).await.map(drop).map_err(|e| Error::Send(e.into()))
        })
    }
}

/// Every mail goes to the log, body included: for development only.
#[derive(Clone)]
pub struct LogMailer {
    from: String,
}

impl LogMailer {
    pub fn new(from: &str) -> Self {
        Self { from: from.trim().to_owned() }
    }
}

impl Mailer for LogMailer {
    fn from(&self) -> &str {
        &self.from
    }

    fn send(&self, message: Message) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let to = mailbox(&message.to)?;
            tracing::info!(to = %to, subject = %message.subject, "mail (log mailer; not sent):\n{}", message.text);
            Ok(())
        })
    }
}

/// Mail is kept in memory; read it with [`sent`](Self::sent). For tests.
/// Clones share what was sent.
#[derive(Clone)]
pub struct MemoryMailer {
    from: String,
    sent: Arc<Mutex<Vec<Message>>>,
}

impl MemoryMailer {
    pub fn new(from: &str) -> Self {
        Self { from: from.trim().to_owned(), sent: Arc::default() }
    }

    /// What was sent, oldest first.
    pub fn sent(&self) -> Vec<Message> {
        self.sent.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Mailer for MemoryMailer {
    fn from(&self) -> &str {
        &self.from
    }

    fn send(&self, message: Message) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            mailbox(&message.to)?;
            self.sent.lock().unwrap_or_else(PoisonError::into_inner).push(message);
            Ok(())
        })
    }
}

/// `[mail]` settings, with feature `configured`:
///
/// ```toml
/// [mail]
/// url = "secret_env(MAIL_URL, log)"
/// from = "env(MAIL_FROM, Acme <hello@acme.test>)"
/// queue = true   # send from `queue:work`; false sends during the request
/// ```
#[cfg(feature = "configured")]
#[derive(Clone, Debug, configured::serde::Deserialize)]
#[serde(crate = "configured::serde", default, deny_unknown_fields)]
pub struct Config {
    /// `smtp://user:pass@host:587?tls=required`, `smtps://...`, `log` or `memory`. A secret: it may hold a password.
    pub url: configured::SecretString,
    pub from: String,
    /// Send through the queue (a worker must run) instead of during the request.
    pub queue: bool,
}

/// `log`, from `hello@example.test`, queued.
#[cfg(feature = "configured")]
impl Default for Config {
    fn default() -> Self {
        Self { url: "log".into(), from: "hello@example.test".into(), queue: true }
    }
}

/// [`from_url`] with the `[mail]` settings.
#[cfg(feature = "configured")]
pub fn from_config(cfg: &Config) -> Result<Box<dyn Mailer>, Error> {
    use configured::ExposeSecret;
    from_url(cfg.url.expose_secret(), &cfg.from)
}
