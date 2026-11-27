//! Published by `art publish auth`. App code: change it freely.

use serde::Deserialize;
use validate::{Messages, Validate};

#[derive(Deserialize, Validate)]
#[validate(messages = messages)]
pub struct ChangePassword {
    #[validate(required)]
    pub current_password: String,
    #[validate(required, min = 8)]
    pub password: String,
    #[validate(same = password)]
    pub password_confirmation: String,
}

fn messages() -> Messages {
    Messages::new().set("password_confirmation.same", "The password confirmation does not match.")
}
