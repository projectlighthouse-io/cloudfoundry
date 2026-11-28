//! Published by `art publish auth`. App code: change it freely.

use serde::Deserialize;
use validate::Validate;

#[derive(Deserialize, Validate)]
pub struct ConfirmPassword {
    #[validate(required)]
    pub password: String,
}
