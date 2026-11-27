//! Rules chained on one field: name the field and value once.
//!
//! ```
//! use validate::{Errors, Validate, Message, rule};
//!
//! #[derive(Debug)]
//! struct NewPipeline { name: String, repo: String, bio: Option<String>, retries: u32 }
//!
//! fn repo_slug(s: &String) -> Result<(), Message> {
//!     rule(s.split_once('/').is_some_and(|(o, n)| !o.is_empty() && !n.is_empty()), "Use owner/name.")
//! }
//!
//! impl Validate for NewPipeline {
//!     fn validate(&self, e: &mut Errors) {
//!         e.field("name", &self.name).required().max_len(80);
//!         e.field("repo", &self.repo).required().rule(repo_slug);
//!         e.optional("bio", self.bio.as_ref()).max_len(500);
//!         e.field("retries", &self.retries).between(0, 10);
//!     }
//! }
//!
//! let p = NewPipeline { name: String::new(), repo: "acme".into(), bio: None, retries: 3 };
//! let e = validate::Valid::check(p).unwrap_err();
//! assert_eq!(e.get("name").len(), 1, "stops at the first failure");
//! assert_eq!(e.get("repo")[0], "Use owner/name.");
//! ```

use std::fmt;

use crate::{Errors, Message, rules};

/// One field's rules, run in order until the first failure. Built by
/// [`Errors::field`] and [`Errors::optional`].
pub struct Field<'e, 'v, T: ?Sized> {
    errors: &'e mut Errors,
    name: &'v str,
    value: Option<&'v T>,
    failed: bool,
}

impl Errors {
    /// Chain rules on `field`. The chain stops at the first rule that fails,
    /// so each field reports one message.
    pub fn field<'e, 'v, T: ?Sized>(&'e mut self, field: &'v str, value: &'v T) -> Field<'e, 'v, T> {
        Field { errors: self, name: field, value: Some(value), failed: false }
    }

    /// Like [`field`](Self::field), but `None` passes every rule: for fields
    /// that may be left out.
    pub fn optional<'e, 'v, T: ?Sized>(&'e mut self, field: &'v str, value: Option<&'v T>) -> Field<'e, 'v, T> {
        Field { errors: self, name: field, value, failed: false }
    }
}

impl<T: ?Sized> Field<'_, '_, T> {
    /// Run any rule: one of this crate's functions or your own.
    pub fn rule(mut self, f: impl FnOnce(&T) -> Result<(), Message>) -> Self {
        if let (false, Some(v)) = (self.failed, self.value) {
            if let Err(m) = f(v) {
                self.errors.add(self.name, m);
                self.failed = true;
            }
        }
        self
    }
}

/// Generates a chain method per text rule: `.email()` runs `rules::email`.
macro_rules! text_rules {
    ($($(#[$m:meta])* $name:ident($($arg:ident: $ty:ty),*);)*) => {
        impl<T: AsRef<str> + ?Sized> Field<'_, '_, T> {
            $($(#[$m])* pub fn $name(self, $($arg: $ty),*) -> Self {
                self.rule(|v| rules::$name(v.as_ref(), $($arg),*))
            })*
        }
    };
}

text_rules! {
    /// Not empty after trimming.
    required();
    min_len(min: usize);
    max_len(max: usize);
    exact_len(len: usize);
    email();
    url();
    uuid();
    ip();
    ipv4();
    ipv6();
    date();
    integer();
    numeric();
    alpha();
    alpha_num();
    alpha_dash();
    lowercase();
    uppercase();
    starts_with(prefix: &str);
    ends_with(suffix: &str);
}

impl<N: PartialOrd + fmt::Display + Copy> Field<'_, '_, N> {
    pub fn between(self, min: N, max: N) -> Self {
        self.rule(|v| rules::between(v, min, max))
    }

    pub fn at_least(self, min: N) -> Self {
        self.rule(|v| rules::at_least(v, min))
    }

    pub fn at_most(self, max: N) -> Self {
        self.rule(|v| rules::at_most(v, max))
    }
}

impl<I> Field<'_, '_, [I]> {
    pub fn min_items(self, min: usize) -> Self {
        self.rule(|v| rules::min_items(v, min))
    }

    pub fn max_items(self, max: usize) -> Self {
        self.rule(|v| rules::max_items(v, max))
    }
}

impl<I> Field<'_, '_, Vec<I>> {
    pub fn min_items(self, min: usize) -> Self {
        self.rule(|v| rules::min_items(v, min))
    }

    pub fn max_items(self, max: usize) -> Self {
        self.rule(|v| rules::max_items(v, max))
    }
}

#[cfg(test)]
mod tests {
    use crate::{Errors, OrMessage, rules};

    #[test]
    fn stops_at_the_first_failure_per_field() {
        let mut e = Errors::new();
        e.field("name", "").required().min_len(3).max_len(5);
        e.field("code", "toolong").required().max_len(3);
        e.field("ok", "fine").required().max_len(10);
        assert_eq!(e.get("name"), ["This field is required."]);
        assert_eq!(e.get("code").len(), 1);
        assert!(e.get("ok").is_empty());
    }

    #[test]
    fn optional_fields_pass_when_absent() {
        let mut e = Errors::new();
        e.optional::<str>("bio", None).required().max_len(1);
        e.optional("site", Some("nope")).url();
        assert!(e.get("bio").is_empty());
        assert_eq!(e.get("site").len(), 1);
    }

    #[test]
    fn numbers_lists_and_custom_rules() {
        let mut e = Errors::new();
        e.field("age", &12u8).at_least(18);
        e.field("tags", &vec!["a"; 6]).max_items(5);
        e.field("plan", "gold").rule(|v| rules::one_of(v, &["free", "pro"]).or_message("Pick a plan."));
        assert_eq!(e.iter().count(), 3);
        assert_eq!(e.get("plan"), ["Pick a plan."]);
    }
}
