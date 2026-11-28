//! Published by `art publish factor2`. App code: change it freely.

use serde::Deserialize;
use validate::Validate;

/// The first code from the authenticator app, to turn two-factor on.
#[derive(Deserialize, Validate)]
pub struct ConfirmCode {
    #[validate(required)]
    pub code: String,
}

/// The login challenge: a code from the app, or a backup code.
#[derive(Deserialize, Validate)]
pub struct Challenge {
    pub code: Option<String>,
    pub backup_code: Option<String>,
    /// Skip the challenge in this browser for 30 days.
    #[serde(default)]
    pub trust: bool,
}
