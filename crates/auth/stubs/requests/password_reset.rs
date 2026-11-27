//! Published by `art publish auth`. App code: change it freely.

use serde::Deserialize;
use validate::{Messages, Validate};

#[derive(Deserialize, Validate)]
pub struct ForgotPassword {
    #[validate(required, email)]
    pub email: String,
}

#[derive(Deserialize, Validate)]
#[validate(messages = messages)]
pub struct ResetPassword {
    #[validate(required)]
    pub token: String,
    #[validate(required, email)]
    pub email: String,
    #[validate(required, min = 8)]
    pub password: String,
    #[validate(same = password)]
    pub password_confirmation: String,
}

fn messages() -> Messages {
    Messages::new().set("password_confirmation.same", "The password confirmation does not match.")
}
