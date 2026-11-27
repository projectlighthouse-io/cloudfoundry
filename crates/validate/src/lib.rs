//! Prove that input was validated.
//!
//! Put rules on a struct's fields with `#[derive(Validate)]`. [`Valid<T>`] can
//! only be built by running them, so a function that takes a `Valid<T>` knows
//! the rules passed. [`Errors`] collects every failure, keyed by the field
//! names the client sent (`#[serde(rename)]` included); each field reports its
//! first failure.
//!
//! # Validating
//!
//! ```
//! use serde::Deserialize;
//! use validate::{Valid, Validate};
//!
//! #[derive(Deserialize, Validate)]
//! struct CreateUser {
//!     #[validate(required, max = 80)]
//!     name: String,
//!     #[validate(required, email)]
//!     email: String,
//!     #[validate(nullable, url)]
//!     website: Option<String>,     // `None` passes; a present value must be a URL
//!     #[serde(rename = "acceptTerms")]
//!     #[validate(accepted)]
//!     terms: bool,                 // errors are keyed "acceptTerms"
//! }
//!
//! let input = CreateUser { name: "".into(), email: "not-an-email".into(), website: None, terms: false };
//! let errors = Valid::check(input).err().unwrap();
//! assert_eq!(errors.get("name"), ["This field is required."]);
//! assert_eq!(errors.get("email"), ["Must be a valid email address."]);
//! assert_eq!(errors.get("acceptTerms"), ["Must be accepted."]);
//! ```
//!
//! # Rules
//!
//! | Attribute | Checks |
//! | --- | --- |
//! | `required` | Text not blank, list not empty, `Option` is `Some` |
//! | `nullable` | Documents an `Option` field: `None` passes every other rule |
//! | `min = n`, `max = n`, `between(a, b)` | Characters for text, items for lists, the value for numbers |
//! | `email`, `url`, `uuid`, `date` | Text shapes (`date` is `YYYY-MM-DD`) |
//! | `integer`, `numeric` | Numbers written as text |
//! | `one_of("a", "b")` | One of a fixed set, for text or numbers |
//! | `same = other_field` | Equal to another field, e.g. a password confirmation |
//! | `accepted` | `true`, for "I accept" checkboxes |
//! | `rule = MyRule` | Your own [`Rule`] |
//! | `with = my_function` | A function given the whole struct and the field: `fn(&Self, &Field) -> Result<(), Message>` |
//!
//! Lengths count characters, not bytes. `String` fields reach rules and
//! functions as `&str`.
//!
//! # Your own rules
//!
//! Implement [`Rule`] for any type, then use it like a built-in one:
//!
//! ```
//! use serde::Deserialize;
//! use validate::{Message, Rule, Valid, Validate, rule};
//!
//! struct RepoSlug;
//!
//! impl Rule<str> for RepoSlug {
//!     fn name(&self) -> &str { "repo_slug" }
//!     fn check(&self, value: &str) -> Result<(), Message> {
//!         rule(value.split_once('/').is_some_and(|(o, n)| !o.is_empty() && !n.is_empty()), "Use owner/name.")
//!     }
//! }
//!
//! #[derive(Deserialize, Validate)]
//! struct NewPipeline {
//!     #[validate(required, rule = RepoSlug)]
//!     repo: String,
//! }
//!
//! assert_eq!(Valid::check(NewPipeline { repo: "acme".into() }).err().unwrap().get("repo"), ["Use owner/name."]);
//! ```
//!
//! # Rules that depend on other fields
//!
//! `with` on a field gets the whole struct; `with` on the struct runs after
//! every field rule and can report errors on any field:
//!
//! ```
//! use serde::Deserialize;
//! use validate::{Errors, Message, Valid, Validate, min, rule};
//!
//! #[derive(Deserialize, Validate)]
//! #[validate(with = admin_rules)]
//! struct NewUser {
//!     admin: bool,
//!     #[validate(required, with = team_rules)]
//!     team: String,
//!     permissions: Vec<String>,
//! }
//!
//! fn team_rules(user: &NewUser, team: &str) -> Result<(), Message> {
//!     if user.admin { rule(team == "staff", "Admins belong to the staff team.") } else { Ok(()) }
//! }
//!
//! fn admin_rules(user: &NewUser, e: &mut Errors) {
//!     if user.admin {
//!         e.field("permissions", &user.permissions).rule(min(1));
//!     }
//! }
//!
//! let e = Valid::check(NewUser { admin: true, team: "ops".into(), permissions: vec![] }).err().unwrap();
//! assert_eq!(e.get("team"), ["Admins belong to the staff team."]);
//! assert_eq!(e.get("permissions"), ["Must have at least 1 item."]);
//! ```
//!
//! # Custom messages
//!
//! Reword chosen rules with a table keyed `field.rule`:
//!
//! ```
//! use serde::Deserialize;
//! use validate::{Messages, Valid, Validate};
//!
//! #[derive(Deserialize, Validate)]
//! #[validate(messages = messages)]
//! struct NewPipeline {
//!     #[validate(required, max = 80)]
//!     name: String,
//! }
//!
//! fn messages() -> Messages {
//!     Messages::new().set("name.required", "Name must be provided for this action.")
//! }
//!
//! let e = Valid::check(NewPipeline { name: "".into() }).err().unwrap();
//! assert_eq!(e.get("name"), ["Name must be provided for this action."]);
//! ```
//!
//! # In handlers (feature `route`)
//!
//! `Valid<Form<T>>`, `Valid<Json<T>>`, `Valid<Query<T>>` and `Valid<Path<T>>` are
//! handler inputs. Invalid input never reaches the handler: the client gets a
//! `422` with every error.
//!
//! ```json
//! { "message": "The given data was invalid.", "errors": { "email": ["Must be a valid email address."] } }
//! ```
//!
//! With `inertia` (feature `validate`), an Inertia visit gets a redirect back
//! instead, and the next page receives the first message per field as
//! `props.errors`.
//!
//! # By hand
//!
//! Without the derive, implement [`Validate`] and use [`Errors::field`]:
//! `e.field("name", &self.name).rule(required()).rule(max(80))`.
//!
//! # What does not belong here
//!
//! Rules that need the database ("email is taken") or the logged-in user run in
//! the handler, after `Valid<T>`: validation here is pure and synchronous.
//! Report the result the same way, with [`Errors::add`] and `errors.into_response()`.

#[cfg(feature = "route")]
mod extract;
mod field;
pub mod rules;

pub use field::Field;
#[cfg(feature = "derive")]
pub use validate_derive::Validate;
pub use rules::*;

use std::{borrow::Cow, collections::BTreeMap, fmt, ops::Deref};

use serde::Serialize;

/// A rule failure, shown to the user.
pub type Message = Cow<'static, str>;

/// An input that knows its own rules.
pub trait Validate {
    /// Run every rule, recording each failure in `e`.
    fn validate(&self, e: &mut Errors);

    /// Your wording for chosen rules, keyed `field.rule`
    /// (`"name.required"`). Rules not listed keep their default message.
    fn messages(&self) -> Messages {
        Messages::default()
    }
}

/// Custom messages for a [`Validate`] type, keyed `field.rule`:
/// `Messages::new().set("name.required", "Name must be provided for this action.")`.
/// The rule part is the rule's [`name`](Rule::name): `required`, `max`, `email`,
/// or the name of your own rule.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Messages(BTreeMap<String, Message>);

impl Messages {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(mut self, key: &str, message: impl Into<Message>) -> Self {
        self.0.insert(key.to_owned(), message.into());
        self
    }
}

/// Every failed rule, keyed by the field name the client sent.
/// Serializes as `{"field": ["message", ...]}`.
#[derive(Clone, Debug, Default)]
pub struct Errors {
    errors: BTreeMap<String, Vec<Message>>,
    messages: Messages,
}

impl PartialEq for Errors {
    fn eq(&self, other: &Self) -> bool {
        self.errors == other.errors
    }
}

impl Eq for Errors {}

impl Serialize for Errors {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.errors.serialize(s)
    }
}

impl Errors {
    pub fn new() -> Self {
        Self::default()
    }

    /// Errors that report `messages` in place of chosen rules' defaults.
    pub fn with_messages(messages: Messages) -> Self {
        Self { errors: BTreeMap::new(), messages }
    }

    /// Report `field` as missing, as the `required` rule would; used by
    /// `#[derive(Validate)]` for an `Option` field that is `None`.
    pub fn missing(&mut self, field: &str) {
        self.fail(field, Some("required"), "This field is required.".into());
    }

    /// Record a failure of the rule `rule` on `field`: the custom message for
    /// `field.rule` if one is set, otherwise `default`.
    pub(crate) fn fail(&mut self, field: &str, rule: Option<&str>, default: Message) {
        let custom = rule.and_then(|r| self.messages.0.get(&format!("{field}.{r}")).cloned());
        self.add(field, custom.unwrap_or(default));
    }

    /// Record `result` for `field` if it is an error.
    pub fn check(&mut self, field: &str, result: Result<(), Message>) -> &mut Self {
        if let Err(m) = result {
            self.add(field, m);
        }
        self
    }

    pub fn add(&mut self, field: &str, message: impl Into<Message>) -> &mut Self {
        self.errors.entry(field.to_owned()).or_default().push(message.into());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn get(&self, field: &str) -> &[Message] {
        self.errors.get(field).map_or(&[], Vec::as_slice)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &[Message])> {
        self.errors.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }
}

impl fmt::Display for Errors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (field, messages) in self.iter() {
            for m in messages {
                if !first {
                    f.write_str("; ")?;
                }
                write!(f, "{field}: {m}")?;
                first = false;
            }
        }
        Ok(())
    }
}

impl std::error::Error for Errors {}

/// A value whose validation ran and passed. There is no other way to build one.
#[derive(Debug)]
pub struct Valid<T>(T);

impl<T: Validate> Valid<T> {
    /// Validate `value`. All rules run; the error holds every failure.
    pub fn check(value: T) -> Result<Self, Errors> {
        let mut e = Errors::with_messages(value.messages());
        value.validate(&mut e);
        if e.is_empty() { Ok(Valid(value)) } else { Err(e) }
    }
}

impl<T> Valid<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Valid<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_every_failure() {
        #[derive(Debug)]
        struct In(String);
        impl Validate for In {
            fn validate(&self, e: &mut Errors) {
                e.field("v", self.0.as_str()).rule(required()).rule(min(2));
                e.check("w", rule(self.0.len() < 10, "Too long."));
            }
        }
        let e = Valid::check(In(String::new())).unwrap_err();
        assert_eq!(e.get("v"), ["This field is required."], "a field stops at its first failure");
        assert_eq!(Valid::check(In("ok".into())).unwrap().into_inner().0, "ok");
    }
}
