//! Published by `art publish auth`. App code: change it freely.

use serde::Deserialize;
use validate::{Errors, OrMessage, Validate, email, min_len, required, same};

#[derive(Deserialize)]
pub struct ForgotPassword {
    pub email: String,
}

impl Validate for ForgotPassword {
    fn validate(&self, e: &mut Errors) {
        e.check("email", required(&self.email)).check("email", email(&self.email));
    }
}

#[derive(Deserialize)]
pub struct ResetPassword {
    pub token: String,
    pub email: String,
    pub password: String,
    pub password_confirmation: String,
}

impl Validate for ResetPassword {
    fn validate(&self, e: &mut Errors) {
        e.check("token", required(&self.token));
        e.check("email", required(&self.email)).check("email", email(&self.email));
        e.check("password", min_len(&self.password, 8))
            .check("password", same(&self.password_confirmation, &self.password).or_message("The password confirmation does not match."));
    }
}
