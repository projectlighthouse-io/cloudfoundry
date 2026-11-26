//! Published by `art publish auth`. App code: change it freely.

use serde::Deserialize;
use validate::{Errors, Validate, email, required};

#[derive(Deserialize)]
pub struct Login {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}

impl Validate for Login {
    fn validate(&self, e: &mut Errors) {
        e.check("email", required(&self.email)).check("email", email(&self.email));
        e.check("password", required(&self.password));
    }
}
