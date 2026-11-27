//! Published by `art publish auth`. App code: change it freely.

use serde::Deserialize;
use validate::Validate;

#[derive(Deserialize, Validate)]
pub struct Login {
    #[validate(required, email)]
    pub email: String,
    #[validate(required)]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}
