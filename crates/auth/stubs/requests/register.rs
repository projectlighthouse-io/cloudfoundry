//! Published by `art publish auth`. App code: change it freely.

use serde::Deserialize;
use validate::{Messages, Validate};

#[derive(Deserialize, Validate)]
#[validate(messages = messages)]
pub struct Register {
    /// Optional. Make it required here (`#[validate(required, max = 255)]`) if your app needs it.
    #[serde(default)]
    #[validate(nullable, max = 255)]
    pub name: Option<String>,
    #[validate(required, email, max = 191)]
    pub email: String,
    #[validate(required, min = 8)]
    pub password: String,
    #[validate(same = password)]
    pub password_confirmation: String,
}

fn messages() -> Messages {
    Messages::new().set("password_confirmation.same", "The password confirmation does not match.")
}
