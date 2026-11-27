//! Rules on one field, by hand. `#[derive(Validate)]` writes these calls for
//! you; write them yourself in a `#[validate(with = ...)]` function for rules
//! that depend on other fields.
//!
//! ```
//! use validate::{Errors, max, min, required};
//!
//! let mut e = Errors::new();
//! e.field("name", "").rule(required()).rule(max(80));   // stops at `required`
//! e.field("permissions", &Vec::<String>::new()).rule(min(1));
//! assert_eq!(e.get("name"), ["This field is required."]);
//! assert_eq!(e.get("permissions"), ["Must have at least 1 item."]);
//! ```

use crate::{Errors, Rule};

/// One field's rules, run in order until the first failure. Built by
/// [`Errors::field`] and [`Errors::optional`].
pub struct Field<'e, 'v, T: ?Sized> {
    errors: &'e mut Errors,
    name: &'v str,
    value: Option<&'v T>,
    failed: bool,
}

impl Errors {
    /// Check rules on `field`. The chain stops at the first rule that fails,
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
    /// Run `rule` unless an earlier rule on this field failed. A failure is
    /// reported under the field, worded by [`Validate::messages`](crate::Validate::messages)
    /// for `"<field>.<rule name>"` if set there.
    pub fn rule(mut self, rule: impl Rule<T>) -> Self {
        if let (false, Some(v)) = (self.failed, self.value) {
            if let Err(m) = rule.check(v) {
                self.errors.fail(self.name, Some(rule.name()), m);
                self.failed = true;
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::{Errors, Messages, between, check_with, max, min, one_of, required, rule, url};

    #[test]
    fn stops_at_the_first_failure_per_field() {
        let mut e = Errors::new();
        e.field("name", "").rule(required()).rule(min(3)).rule(max(5));
        e.field("code", "toolong").rule(required()).rule(max(3));
        e.field("ok", "fine").rule(required()).rule(max(10));
        assert_eq!(e.get("name"), ["This field is required."]);
        assert_eq!(e.get("code"), ["Must be at most 3 characters."]);
        assert!(e.get("ok").is_empty());
    }

    #[test]
    fn optional_fields_pass_when_absent() {
        let mut e = Errors::new();
        e.optional::<str>("bio", None).rule(required()).rule(max(1));
        e.optional("site", Some("nope")).rule(url());
        assert!(e.get("bio").is_empty());
        assert_eq!(e.get("site"), ["Must be a valid URL."]);
    }

    #[test]
    fn numbers_lists_closures_and_messages() {
        let mut e = Errors::with_messages(Messages::new().set("plan.one_of", "Pick a plan.").set("slug.slug", "Use owner/name, e.g. acme/shop."));
        e.field("age", &12u8).rule(between(18, 120));
        e.field("tags", &vec!["a"; 6]).rule(max(5));
        e.field("plan", "gold").rule(one_of(["free", "pro"]));
        e.field("slug", "acme").rule(check_with("slug", |s: &str| rule(s.contains('/'), "Use owner/name.")));
        assert_eq!(e.get("age"), ["Must be between 18 and 120."]);
        assert_eq!(e.get("tags"), ["Must have at most 5 items."]);
        assert_eq!(e.get("plan"), ["Pick a plan."]);
        assert_eq!(e.get("slug"), ["Use owner/name, e.g. acme/shop."]);
    }
}
