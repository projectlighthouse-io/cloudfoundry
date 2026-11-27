//! The built-in rules, and the [`Rule`] trait for writing your own.
//!
//! Each rule is a small value: `required()`, `max(255)`, `one_of(["free", "pro"])`.
//! Attach them with `#[validate(...)]` (feature `derive`) or, inside a
//! hand-written check, with `e.field("name", &value).rule(max(255))`.

use std::fmt;

use crate::Message;

/// A validation rule for values of type `T`.
///
/// ```
/// use validate::{Message, Rule, rule};
///
/// struct RepoSlug;
///
/// impl Rule<str> for RepoSlug {
///     fn name(&self) -> &str { "repo_slug" }   // reworded in messages() as "<field>.repo_slug"
///     fn check(&self, value: &str) -> Result<(), Message> {
///         rule(value.split_once('/').is_some_and(|(o, n)| !o.is_empty() && !n.is_empty()), "Use owner/name.")
///     }
/// }
/// ```
pub trait Rule<T: ?Sized> {
    /// The rule's key in [`Messages`](crate::Messages): `"<field>.<name>"`.
    fn name(&self) -> &str;
    fn check(&self, value: &T) -> Result<(), Message>;
}

/// A rule from a condition: `rule(input.starts_with('#'), "Must start with #.")`.
pub fn rule(ok: bool, message: impl Into<Message>) -> Result<(), Message> {
    if ok { Ok(()) } else { Err(message.into()) }
}

/// Replace a rule result's message: `check(...).or_message("Tell us your name.")`.
pub trait OrMessage {
    fn or_message(self, message: impl Into<Message>) -> Self;
}

impl OrMessage for Result<(), Message> {
    fn or_message(self, message: impl Into<Message>) -> Self {
        self.map_err(|_| message.into())
    }
}

/// Values `required` understands: text is present when it is not blank, a
/// list when it is not empty, an `Option` when it is `Some` with a present value.
pub trait Presence {
    fn is_present(&self) -> bool;
}

impl Presence for str {
    fn is_present(&self) -> bool {
        !self.trim().is_empty()
    }
}

impl Presence for String {
    fn is_present(&self) -> bool {
        self.as_str().is_present()
    }
}

impl<I> Presence for Vec<I> {
    fn is_present(&self) -> bool {
        !self.is_empty()
    }
}

impl<I> Presence for [I] {
    fn is_present(&self) -> bool {
        !self.is_empty()
    }
}

impl<T: Presence> Presence for Option<T> {
    fn is_present(&self) -> bool {
        self.as_ref().is_some_and(Presence::is_present)
    }
}

/// What `min`, `max` and `between` compare: the length of text, the number of
/// items in a list, or the value of a number.
pub trait Measure {
    fn measure(&self) -> (f64, Unit);
}

/// The unit a [`Measure`] is in, for messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Characters,
    Items,
    Value,
}

impl Measure for str {
    fn measure(&self) -> (f64, Unit) {
        (self.chars().count() as f64, Unit::Characters)
    }
}

impl Measure for String {
    fn measure(&self) -> (f64, Unit) {
        self.as_str().measure()
    }
}

impl<I> Measure for Vec<I> {
    fn measure(&self) -> (f64, Unit) {
        (self.len() as f64, Unit::Items)
    }
}

impl<I> Measure for [I] {
    fn measure(&self) -> (f64, Unit) {
        (self.len() as f64, Unit::Items)
    }
}

/// Numbers that `min`, `max`, `between` and `one_of` accept.
pub trait Number: Copy + PartialEq + fmt::Display {
    fn as_f64(self) -> f64;
}

macro_rules! numbers {
    ($($t:ty),*) => {$(
        impl Number for $t {
            fn as_f64(self) -> f64 {
                self as f64
            }
        }
        impl Measure for $t {
            fn measure(&self) -> (f64, Unit) {
                (*self as f64, Unit::Value)
            }
        }
    )*};
}

numbers!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize, f32, f64);

fn show(n: f64) -> String {
    if n.fract() == 0.0 { format!("{}", n as i64) } else { n.to_string() }
}

fn plural(n: f64, one: &str, many: &str) -> String {
    format!("{} {}", show(n), if n == 1.0 { one } else { many })
}

/// Present: text that is not blank, a non-empty list, or `Some`.
pub struct Required;

pub fn required() -> Required {
    Required
}

impl<T: Presence + ?Sized> Rule<T> for Required {
    fn name(&self) -> &str {
        "required"
    }
    fn check(&self, value: &T) -> Result<(), Message> {
        rule(value.is_present(), "This field is required.")
    }
}

/// At least `n`: characters for text, items for lists, the value for numbers.
pub struct Min(f64);

pub fn min(n: impl Number) -> Min {
    Min(n.as_f64())
}

impl<T: Measure + ?Sized> Rule<T> for Min {
    fn name(&self) -> &str {
        "min"
    }
    fn check(&self, value: &T) -> Result<(), Message> {
        let (v, unit) = value.measure();
        rule(v >= self.0, match unit {
            Unit::Characters => format!("Must be at least {}.", plural(self.0, "character", "characters")),
            Unit::Items => format!("Must have at least {}.", plural(self.0, "item", "items")),
            Unit::Value => format!("Must be at least {}.", show(self.0)),
        })
    }
}

/// At most `n`: characters for text, items for lists, the value for numbers.
pub struct Max(f64);

pub fn max(n: impl Number) -> Max {
    Max(n.as_f64())
}

impl<T: Measure + ?Sized> Rule<T> for Max {
    fn name(&self) -> &str {
        "max"
    }
    fn check(&self, value: &T) -> Result<(), Message> {
        let (v, unit) = value.measure();
        rule(v <= self.0, match unit {
            Unit::Characters => format!("Must be at most {}.", plural(self.0, "character", "characters")),
            Unit::Items => format!("Must have at most {}.", plural(self.0, "item", "items")),
            Unit::Value => format!("Must be at most {}.", show(self.0)),
        })
    }
}

/// From `low` to `high`, inclusive, measured like [`min`] and [`max`].
pub struct Between(f64, f64);

pub fn between(low: impl Number, high: impl Number) -> Between {
    Between(low.as_f64(), high.as_f64())
}

impl<T: Measure + ?Sized> Rule<T> for Between {
    fn name(&self) -> &str {
        "between"
    }
    fn check(&self, value: &T) -> Result<(), Message> {
        let (v, unit) = value.measure();
        let (lo, hi) = (show(self.0), show(self.1));
        rule((self.0..=self.1).contains(&v), match unit {
            Unit::Characters => format!("Must be between {lo} and {hi} characters."),
            Unit::Items => format!("Must have between {lo} and {hi} items."),
            Unit::Value => format!("Must be between {lo} and {hi}."),
        })
    }
}

/// Text shape checks, all on `str`.
macro_rules! text_rule {
    ($(#[$m:meta])* $ty:ident, $fun:ident, $name:literal, $message:literal, |$s:ident| $ok:expr) => {
        $(#[$m])*
        pub struct $ty;

        $(#[$m])*
        pub fn $fun() -> $ty {
            $ty
        }

        impl<T: AsRef<str> + ?Sized> Rule<T> for $ty {
            fn name(&self) -> &str {
                $name
            }
            fn check(&self, value: &T) -> Result<(), Message> {
                let $s: &str = value.as_ref();
                rule($ok, $message)
            }
        }
    };
}

text_rule!(
    /// Shaped like an email address: one `@`, a local part, a dotted domain, no spaces.
    /// Shape only: confirm ownership by sending mail.
    Email, email, "email", "Must be a valid email address.",
    |s| match s.split_once('@') {
        Some((local, domain)) => !local.is_empty()
            && !domain.contains('@')
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && !s.chars().any(char::is_whitespace),
        None => false,
    }
);

text_rule!(
    /// An `http` or `https` URL with a host.
    Url, url, "url", "Must be a valid URL.",
    |s| ["http://", "https://"].iter().any(|p| s.strip_prefix(p).is_some_and(|rest| {
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        !host.is_empty() && !s.chars().any(char::is_whitespace)
    }))
);

text_rule!(
    /// A UUID in its 36-character hyphenated form.
    Uuid, uuid, "uuid", "Must be a valid UUID.",
    |s| s.len() == 36
        && s.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
);

text_rule!(
    /// A calendar date as `YYYY-MM-DD`, leap years included.
    Date, date, "date", "Must be a date as YYYY-MM-DD.",
    |s| {
        let parts: Vec<&str> = s.split('-').collect();
        match parts.as_slice() {
            [y, m, d] if y.len() == 4 && m.len() == 2 && d.len() == 2 => {
                match (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>()) {
                    (Ok(y), Ok(m), Ok(d)) => {
                        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
                        let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
                        (1..=12).contains(&m) && d >= 1 && d <= days[m as usize - 1]
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
);

text_rule!(
    /// A whole number written as text, e.g. a query parameter: `-12`, `42`.
    Integer, integer, "integer", "Must be a whole number.",
    |s| s.trim().parse::<i64>().is_ok()
);

text_rule!(
    /// A number written as text: `3`, `-0.5`, `1e3`.
    Numeric, numeric, "numeric", "Must be a number.",
    |s| s.trim().parse::<f64>().is_ok_and(f64::is_finite)
);

/// One of a fixed set of values: `one_of(["free", "pro"])` for text,
/// `one_of([1, 2, 3])` for numbers.
pub struct OneOf<V>(Vec<V>);

pub fn one_of<V>(values: impl IntoIterator<Item = V>) -> OneOf<V> {
    OneOf(values.into_iter().collect())
}

impl<T: AsRef<str> + ?Sized> Rule<T> for OneOf<&str> {
    fn name(&self) -> &str {
        "one_of"
    }
    fn check(&self, value: &T) -> Result<(), Message> {
        rule(self.0.contains(&value.as_ref()), format!("Must be one of: {}.", self.0.join(", ")))
    }
}

impl<N: Number> Rule<N> for OneOf<N> {
    fn name(&self) -> &str {
        "one_of"
    }
    fn check(&self, value: &N) -> Result<(), Message> {
        let list: Vec<String> = self.0.iter().map(N::to_string).collect();
        rule(self.0.contains(value), format!("Must be one of: {}.", list.join(", ")))
    }
}

/// Equal to another value, e.g. a password confirmation: `same(&self.password)`.
pub struct Same<'a, T: ?Sized>(&'a T);

pub fn same<T: ?Sized>(other: &T) -> Same<'_, T> {
    Same(other)
}

impl<T: PartialEq + ?Sized> Rule<T> for Same<'_, T> {
    fn name(&self) -> &str {
        "same"
    }
    fn check(&self, value: &T) -> Result<(), Message> {
        rule(value == self.0, "Does not match.")
    }
}

/// `true`, for "I accept the terms" checkboxes.
pub struct Accepted;

pub fn accepted() -> Accepted {
    Accepted
}

impl Rule<bool> for Accepted {
    fn name(&self) -> &str {
        "accepted"
    }
    fn check(&self, value: &bool) -> Result<(), Message> {
        rule(*value, "Must be accepted.")
    }
}

/// A one-off rule from a closure, with a name for [`Messages`](crate::Messages):
/// `check_with("slug", |s: &str| rule(s.contains('/'), "Use owner/name."))`.
pub struct Check<F>(&'static str, F);

pub fn check_with<F>(name: &'static str, f: F) -> Check<F> {
    Check(name, f)
}

impl<T: ?Sized, F: Fn(&T) -> Result<(), Message>> Rule<T> for Check<F> {
    fn name(&self) -> &str {
        self.0
    }
    fn check(&self, value: &T) -> Result<(), Message> {
        (self.1)(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok<T: ?Sized, R: Rule<T>>(r: R, v: &T) -> bool {
        r.check(v).is_ok()
    }

    #[test]
    fn required_understands_text_lists_and_options() {
        assert!(!ok(required(), "  "));
        assert!(ok(required(), "a"));
        assert!(!ok(required(), &Vec::<u8>::new()));
        assert!(!ok(required(), &None::<String>));
        assert!(!ok(required(), &Some(String::new())));
        assert!(ok(required(), &Some("x".to_owned())));
    }

    #[test]
    fn min_max_between_measure_each_kind() {
        assert_eq!(max(3).check("abcd").unwrap_err(), "Must be at most 3 characters.");
        assert_eq!(min(1).check(&Vec::<u8>::new()).unwrap_err(), "Must have at least 1 item.");
        assert_eq!(between(1, 10).check(&11u32).unwrap_err(), "Must be between 1 and 10.");
        assert!(ok(max(3), "héé"), "characters, not bytes");
        assert!(ok(between(0.5, 1.5), &1.0f64));
    }

    #[test]
    fn shapes() {
        assert!(ok(email(), "ada@example.com") && !ok(email(), "ada@example") && !ok(email(), "a b@x.io"));
        assert!(ok(url(), "https://toolkit.dev/docs") && !ok(url(), "ftp://x.io") && !ok(url(), "https://"));
        assert!(ok(uuid(), "123e4567-e89b-12d3-a456-426614174000") && !ok(uuid(), "123e4567"));
        assert!(ok(date(), "2024-02-29") && !ok(date(), "2023-02-29") && !ok(date(), "2024-13-01"));
        assert!(ok(integer(), "-12") && !ok(integer(), "1.5"));
        assert!(ok(numeric(), "-0.5") && !ok(numeric(), "NaN") && !ok(numeric(), "abc"));
    }

    #[test]
    fn one_of_same_accepted_and_closures() {
        assert!(ok(one_of(["free", "pro"]), "pro"));
        assert_eq!(one_of(["free", "pro"]).check("gold").unwrap_err(), "Must be one of: free, pro.");
        assert!(ok(one_of([1, 2]), &2) && !ok(one_of([1, 2]), &3));
        assert!(ok(same("pw"), "pw") && !ok(same("pw"), "px"));
        assert!(ok(accepted(), &true) && !ok(accepted(), &false));
        let slug = check_with("slug", |s: &str| rule(s.contains('/'), "Use owner/name."));
        assert_eq!(Rule::<str>::name(&slug), "slug");
        assert!(ok(slug, "a/b"));
    }

    #[test]
    fn text_rules_check_strings_too() {
        assert!(!ok(email(), &String::from("nope")));
        assert!(ok(one_of(["a"]), &String::from("a")));
    }
}
