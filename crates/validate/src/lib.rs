//! Prove that input was validated.
//!
//! ```
//! use validate::{Errors, Valid, Validate, max_len, required};
//!
//! struct CreatePipeline { name: String }
//!
//! impl Validate for CreatePipeline {
//!     fn validate(&self, e: &mut Errors) {
//!         e.check("name", required(&self.name));
//!         e.check("name", max_len(&self.name, 80));
//!     }
//! }
//!
//! let err = Valid::check(CreatePipeline { name: String::new() }).err().unwrap();
//! assert_eq!(err.get("name"), ["This field is required."]);
//! ```

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
