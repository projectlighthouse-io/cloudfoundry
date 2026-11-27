//! Published by `art publish auth`. App code: change it freely.

use serde::Deserialize;
use validate::{Errors, OrMessage, Validate, email, max_len, min_len, required, same};

#[derive(Deserialize)]
pub struct Register {
    pub email: String,
    pub password: String,
    pub password_confirmation: String,
}

impl Validate for Register {
    fn validate(&self, e: &mut Errors) {
        e.check("email", required(&self.email)).check("email", email(&self.email)).check("email", max_len(&self.email, 191));
        e.check("password", min_len(&self.password, 8))
            .check("password", same(&self.password_confirmation, &self.password).or_message("The password confirmation does not match."));
    }
}
