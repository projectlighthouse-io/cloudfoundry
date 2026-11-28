//! Published by `art publish mail`. App code: change it freely.
//!
//! The app's one mailer, built from `[mail]` in config on first use, and
//! `deliver`: render a template and send it, through the queue by default.

use std::sync::OnceLock;

use crate::state::App;

static MAILER: OnceLock<Box<dyn mail::Mailer>> = OnceLock::new();

/// A bad `[mail] url` is logged and mail goes to the log instead, so nothing is lost silently.
pub fn mailer(app: &App) -> &'static dyn mail::Mailer {
    MAILER
        .get_or_init(|| {
            mail::from_config(&app.cfg.mail).unwrap_or_else(|e| {
                tracing::error!("[mail] url: {e}; writing mail to the log instead");
                Box::new(mail::LogMailer::new(&app.cfg.mail.from))
            })
        })
        .as_ref()
}

/// Lets `queue:work` send `mail::SendMail` jobs (registered in `src/jobs/mod.rs`).
impl mail::HasMailer for App {
    fn mailer(&self) -> &dyn mail::Mailer {
        mailer(self)
    }
}

/// Render `template` (a `resources/mail/*.md` file) for `to` and send it: from
/// the queue with `[mail] queue = true` (the default; `queue:work` must run),
/// or now. Failures are logged: callers carry on.
pub async fn deliver(app: &App, template: &str, to: &str, vars: &[(&str, &str)]) {
    let message = match mail::Template::parse(template).and_then(|t| t.message(to, vars)) {
        Ok(m) => m,
        Err(e) => return tracing::error!("mail to {to}: {e}"),
    };
    let sent = if app.cfg.mail.queue {
        mail::send_later::<App, _>(&app.queue, message).await.map(drop).map_err(|e| e.to_string())
    } else {
        mailer(app).send(message).await.map_err(|e| e.to_string())
    };
    if let Err(e) = sent {
        tracing::error!("mail to {to}: {e}");
    }
}
