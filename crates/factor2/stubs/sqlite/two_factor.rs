//! Published by `art publish factor2`. App code: change it freely.
//!
//! The app's one `TwoFactor`, built on first use. Secrets are sealed with
//! `crypt` and the trusted-browser cookie signed with `securecookie`, both
//! keyed from APP_KEY (each derives its own key from it).

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use configured::ExposeSecret;

use crate::state::App;

pub type Store = factor2::SqliteStore;
pub type TwoFactor = factor2::TwoFactor<Store>;

static TWO_FACTOR: OnceLock<TwoFactor> = OnceLock::new();

/// Wrong codes: 5 a minute per user and IP. Settings: `factor2::Config`
/// (`.config(...)`), e.g. how long "trust this browser" lasts (30 days).
pub fn two_factor(app: &App) -> &'static TwoFactor {
    TWO_FACTOR.get_or_init(|| {
        let key = app.cfg.app_key.expose_secret();
        // main.rs parsed this key for the session codec, so it is valid here.
        let crypt = crypt::Encrypter::new(crypt::Keys::parse(key).expect("APP_KEY was checked at startup"));
        let cookies = securecookie::Codec::new(securecookie::Keys::parse(key).expect("APP_KEY was checked at startup"));
        let limiter = Arc::new(throttle::Limiter::memory(throttle::FixedWindow::new(5, Duration::from_secs(60))));
        factor2::TwoFactor::new(Store::new(app.db.pool().clone()), crypt, cookies, &app.cfg.app_name).throttle(limiter)
    })
}
