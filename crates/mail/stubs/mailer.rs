//! Published by `art publish mail`. App code: change it freely.
//!
//! The app's one `mail::Mailer`, built from `[mail]` in config on first use.

use std::sync::OnceLock;

use crate::state::App;

static MAILER: OnceLock<mail::Mailer> = OnceLock::new();

/// A bad `[mail] url` is logged and mail goes to the log instead, so nothing is lost silently.
pub fn mailer(app: &App) -> &'static mail::Mailer {
    MAILER.get_or_init(|| {
        mail::Mailer::config(&app.cfg.mail).unwrap_or_else(|e| {
            tracing::error!("[mail] url: {e}; writing mail to the log instead");
            mail::Mailer::log(&app.cfg.mail.from)
        })
    })
}
