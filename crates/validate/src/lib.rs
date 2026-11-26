//! Prove that input was validated.
//!
//! An input type implements [`Validate`]. [`Valid<T>`] can only be built by
//! running that validation, so a function that takes a `Valid<T>` knows the
//! rules passed. Every rule runs; [`Errors`] collects every failure, keyed by
//! the field names the client sent.
//!
//! # Validating
//!
//! ```
//! use validate::{Errors, Valid, Validate, email, max_len, required};
//!
//! struct CreateUser {
//!     name: String,
//!     email: String,
//! }
//!
//! impl Validate for CreateUser {
//!     fn validate(&self, e: &mut Errors) {
//!         e.check("name", required(&self.name)).check("name", max_len(&self.name, 80));
//!         e.check("email", email(&self.email));
//!     }
//! }
//!
//! let input = CreateUser { name: "".into(), email: "not-an-email".into() };
//! let errors = Valid::check(input).err().unwrap();
//! assert_eq!(errors.get("name"), ["This field is required."]);
//! assert_eq!(errors.get("email"), ["Must be a valid email address."]);
//!
//! let ok = Valid::check(CreateUser { name: "Ada".into(), email: "ada@example.com".into() }).ok().unwrap();
//! assert_eq!(ok.name, "Ada"); // `Valid<T>` derefs to `T`; `into_inner()` takes it out
//! ```
//!
//! # In handlers (feature `route`)
//!
//! `Valid<Form<T>>`, `Valid<Json<T>>`, `Valid<Query<T>>` and `Valid<Path<T>>` are
//! handler inputs. Invalid input never reaches the handler: the client gets a
//! `422` with every error.
//!
//! ```ignore
//! async fn store(input: Valid<Json<CreateUser>>) -> Result<Redirect, AppError> {
//!     let route::Json(user) = input.into_inner();
//!     // ...
//! }
//! ```
//!
//! ```json
//! { "message": "The given data was invalid.", "errors": { "email": ["Must be a valid email address."] } }
//! ```
//!
//! With `inertia` (feature `validate`), an Inertia visit gets a redirect back
//! instead, and the next page receives the first message per field as
//! `props.errors`.
//!
//! # Built-in rules
//!
//! | Kind | Rules |
//! | --- | --- |
//! | Presence, length | [`required`], [`min_len`], [`max_len`], [`exact_len`], [`min_items`], [`max_items`] |
//! | Numbers | [`between`], [`at_least`], [`at_most`], [`integer`], [`numeric`] (the last two check text) |
//! | Choices, comparisons | [`one_of`], [`not_one_of`], [`same`], [`different`], [`accepted`] |
//! | Characters | [`alpha`], [`alpha_num`], [`alpha_dash`], [`lowercase`], [`uppercase`], [`starts_with`], [`ends_with`] |
//! | Formats | [`email`], [`url`], [`uuid`], [`ip`], [`ipv4`], [`ipv6`], [`date`] |
//!
//! Lengths count characters, not bytes.
//!
//! # Custom rules
//!
//! A rule is any function returning `Result<(), Message>`, so an app's rules
//! sit next to the built-in ones:
//!
//! ```
//! use validate::{Errors, Message, OrMessage, Valid, Validate, alpha_dash, max_len, nullable, required, rule, same};
//!
//! /// Reusable: `owner/name`.
//! fn repo_slug(s: &str) -> Result<(), Message> {
//!     let parts: Vec<&str> = s.split('/').collect();
//!     rule(parts.len() == 2 && parts.iter().all(|p| alpha_dash(p).is_ok()), "Must look like owner/name.")
//! }
//!
//! struct Signup { name: String, repo: String, password: String, confirm: String, bio: Option<String>, tags: Vec<String> }
//!
//! impl Validate for Signup {
//!     fn validate(&self, e: &mut Errors) {
//!         e.check("name", required(&self.name).or_message("Tell us your name.")); // own message
//!         e.check("repo", repo_slug(&self.repo));                                  // own rule
//!         e.check("confirm", same(&self.password, &self.confirm));                 // cross-field
//!         e.check("bio", nullable(self.bio.as_deref(), |b| max_len(b, 500)));      // optional field
//!         for (i, tag) in self.tags.iter().enumerate() {                           // per item
//!             e.check(&format!("tags.{i}"), required(tag));
//!         }
//!         e.check("tags", rule(self.tags.len() <= 3, "Pick at most 3 tags."));   // one-off
//!     }
//! }
//!
//! let bad = Signup { name: "".into(), repo: "nope".into(), password: "a".into(), confirm: "b".into(), bio: None, tags: vec![] };
//! let e = Valid::check(bad).err().unwrap();
//! assert_eq!(e.get("name"), ["Tell us your name."]);
//! assert_eq!(e.get("repo"), ["Must look like owner/name."]);
//! assert_eq!(e.get("confirm"), ["Must match."]);
//! ```
//!
//! [`Errors::add`] records a message directly, for checks that are not a rule.
//!
//! # What does not belong here
//!
//! Rules that need the database ("email is taken") run in an action, where the
//! connection is: validation here is pure and synchronous. Report the result
//! the same way, with [`Errors::add`] and `errors.into_response()`.

#[cfg(feature = "route")]
mod extract;
pub mod rules;

pub use rules::*;

use std::{borrow::Cow, collections::BTreeMap, fmt, ops::Deref};

use serde::Serialize;

/// A rule failure, shown to the user.
pub type Message = Cow<'static, str>;

/// An input that knows its own rules.
pub trait Validate {
    /// Run every rule, recording each failure in `e`.
    fn validate(&self, e: &mut Errors);
}

/// Every failed rule, keyed by the field name the client sent.
/// Serializes as `{"field": ["message", ...]}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Errors(BTreeMap<String, Vec<Message>>);

impl Errors {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `result` for `field` if it is an error.
    pub fn check(&mut self, field: &str, result: Result<(), Message>) -> &mut Self {
        if let Err(m) = result {
            self.add(field, m);
        }
        self
    }

    pub fn add(&mut self, field: &str, message: impl Into<Message>) -> &mut Self {
        self.0.entry(field.to_owned()).or_default().push(message.into());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, field: &str) -> &[Message] {
        self.0.get(field).map_or(&[], Vec::as_slice)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &[Message])> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
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
        let mut e = Errors::new();
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
                e.check("v", required(&self.0)).check("v", min_len(&self.0, 2));
            }
        }
        let e = Valid::check(In(String::new())).unwrap_err();
        assert_eq!(e.get("v").len(), 2);
        assert_eq!(Valid::check(In("ok".into())).unwrap().into_inner().0, "ok");
    }
}
