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

// Rules. Each is a plain function; apps write their own the same way.

/// Not empty after trimming whitespace.
pub fn required(s: &str) -> Result<(), Message> {
    if s.trim().is_empty() { Err("This field is required.".into()) } else { Ok(()) }
}

/// At least `min` characters.
pub fn min_len(s: &str, min: usize) -> Result<(), Message> {
    if s.chars().count() < min { Err(format!("Must be at least {min} characters.").into()) } else { Ok(()) }
}

/// At most `max` characters.
pub fn max_len(s: &str, max: usize) -> Result<(), Message> {
    if s.chars().count() > max { Err(format!("Must be at most {max} characters.").into()) } else { Ok(()) }
}

/// Within `min..=max`.
pub fn between<N: PartialOrd + fmt::Display>(n: &N, min: N, max: N) -> Result<(), Message> {
    if *n < min || *n > max { Err(format!("Must be between {min} and {max}.").into()) } else { Ok(()) }
}

/// One of `allowed`.
pub fn one_of<T: PartialEq + ?Sized>(value: &T, allowed: &[&T]) -> Result<(), Message> {
    if allowed.contains(&value) { Ok(()) } else { Err("Must be one of the allowed values.".into()) }
}

/// Shaped like an email address: one `@`, a non-empty local part, a dotted
/// domain, no whitespace.
/// ponytail: shape only; confirm ownership by sending mail, not by parsing.
pub fn email(s: &str) -> Result<(), Message> {
    let ok = match s.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && !domain.contains('@')
                && domain.split('.').count() >= 2
                && domain.split('.').all(|p| !p.is_empty())
                && !s.chars().any(char::is_whitespace)
        }
        None => false,
    };
    if ok { Ok(()) } else { Err("Must be a valid email address.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        assert!(required("  ").is_err() && required("x").is_ok());
        assert!(min_len("ab", 3).is_err() && min_len("abc", 3).is_ok());
        assert!(max_len("héllo", 5).is_ok() && max_len("héllo!", 5).is_err());
        assert!(between(&5, 1, 10).is_ok() && between(&11, 1, 10).is_err());
        assert!(one_of("b", &["a", "b"]).is_ok() && one_of("c", &["a", "b"]).is_err());
        for good in ["a@b.co", "x.y+z@mail.example.org"] {
            assert!(email(good).is_ok(), "{good}");
        }
        for bad in ["", "a", "@b.co", "a@b", "a@b.", "a@@b.co", "a b@c.de"] {
            assert!(email(bad).is_err(), "{bad}");
        }
    }

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
