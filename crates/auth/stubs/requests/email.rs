//! Published by `art publish auth`. App code: change it freely.

use serde::Deserialize;
use validate::Validate;

/// A new email address; it gets a verification link first.
#[derive(Deserialize, Validate)]
pub struct ChangeEmail {
    #[validate(required, email)]
    pub email: String,
}
