//! Published by `art publish passkeys`. App code: change it freely.

use passkeys::{PublicKeyCredential, RegisterPublicKeyCredential};
use serde::Deserialize;
use validate::Validate;

/// A new passkey: the browser's `credential.toJSON()` and a name for the list.
#[derive(Deserialize, Validate)]
pub struct AddPasskey {
    #[validate(required, max = 100)]
    pub name: String,
    pub credential: RegisterPublicKeyCredential,
}

#[derive(Deserialize, Validate)]
pub struct RenamePasskey {
    #[validate(required, max = 100)]
    pub name: String,
}

/// A sign-in or second factor: the browser's `credential.toJSON()`.
#[derive(Deserialize, Validate)]
pub struct UsePasskey {
    pub credential: PublicKeyCredential,
    /// Second factor only: skip the two-factor challenge in this browser for 30 days.
    #[serde(default)]
    pub trust: bool,
}
