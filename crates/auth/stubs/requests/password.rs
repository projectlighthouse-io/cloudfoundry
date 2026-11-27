//! Published by `art publish auth`. App code: change it freely.

use serde::Deserialize;
use validate::{Errors, OrMessage, Validate, min_len, required, same};

#[derive(Deserialize)]
pub struct ChangePassword {
    pub current_password: String,
    pub password: String,
    pub password_confirmation: String,
}

impl Validate for ChangePassword {
    fn validate(&self, e: &mut Errors) {
        e.check("current_password", required(&self.current_password));
        e.check("password", min_len(&self.password, 8))
            .check("password", same(&self.password_confirmation, &self.password).or_message("The password confirmation does not match."));
    }
}
