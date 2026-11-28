//! Typed app config from files, with env vars and secrets resolved.
//!
//! The app loads one struct; library crates only expose `Config` structs that
//! deserialize through this crate (`default-features = false`).
//!
//! ```toml
//! # config/default.toml
//! app_name     = "env(APP_NAME, my-app)"
//! database_url = "required_env(DATABASE_URL)"
//! app_key      = "required_secret_env(APP_KEY)"
//!
//! [session]
//! lifetime = "2h"
//! ```
//!
//! ```ignore
//! #[derive(serde::Deserialize)]
//! #[serde(deny_unknown_fields)]
//! struct Config {
//!     app_name: String,
//!     database_url: String,
//!     app_key: configured::SecretString,
//!     #[serde(default)]
//!     session: session::Config,
//! }
//!
//! let loaded = configured::load::<Config>("config")?;
//! println!("{}", loaded.listing.show()); // secrets print as <redacted>
//! let cfg = loaded.value;
//! ```

#[cfg(feature = "load")]
mod load;

#[cfg(feature = "load")]
pub use load::{Error, Listing, Loaded, load};
pub use secrecy::{ExposeSecret, SecretString};
/// For `#[serde(crate = "configured::serde")]`: crates derive through this
/// re-export and need no `serde` dependency of their own.
pub use serde;

/// `Duration` as a humantime string (`"90s"`, `"2h"`, `"14d"`):
/// `#[serde(with = "configured::duration")]`.
pub mod duration {
    use std::time::Duration;

    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        humantime_serde::serialize(d, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        humantime_serde::deserialize(d)
    }
}

/// `Option<Duration>` as `"never"` (`None`) or a humantime string:
/// `#[serde(with = "configured::duration_or_never")]`. Formats without a null
/// (TOML, env vars) could not turn a default `Some` off otherwise.
pub mod duration_or_never {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => humantime_serde::serialize(d, s),
            None => s.serialize_str("never"),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let s = String::deserialize(d)?;
        if s == "never" {
            return Ok(None);
        }
        humantime_serde::re::humantime::parse_duration(&s).map(Some).map_err(serde::de::Error::custom)
    }
}
