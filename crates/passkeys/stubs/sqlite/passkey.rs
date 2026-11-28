//! Published by `art publish passkeys`. App code: change it freely.
//!
//! The app's one `Passkeys`, built on first use. Passkeys are bound to the
//! host of `app_url` (APP_URL): changing it later orphans every passkey.

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use crate::state::App;

pub type Store = passkeys::SqliteStore;
pub type Passkeys = passkeys::Passkeys<Store>;

static PASSKEYS: OnceLock<Passkeys> = OnceLock::new();

/// Failed sign-ins: 5 a minute per IP; failed second factors: 5 a minute per user and IP.
/// Emits `passkeys::events::*` on `app.events` (feature `event`); register listeners in `src/events.rs`.
pub fn passkeys(app: &App) -> &'static Passkeys {
    PASSKEYS.get_or_init(|| {
        let limiter = Arc::new(throttle::Limiter::memory(throttle::FixedWindow::new(5, Duration::from_secs(60))));
        passkeys::Passkeys::new(Store::new(app.db.pool().clone()), &app.cfg.app_url, &app.cfg.app_name)
            .expect("APP_URL must be an absolute URL with a host")
            .throttle(limiter)
            .events(app.events.clone(), app.clone())
    })
}
