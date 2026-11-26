//! Built-in rules. Each is a plain function returning `Result<(), Message>`;
//! an app's own rules are written the same way (see [`rule`]).

use std::{fmt, net::IpAddr};

use crate::Message;

fn check(ok: bool, message: impl FnOnce() -> Message) -> Result<(), Message> {
    if ok { Ok(()) } else { Err(message()) }
}

/// A rule from a condition: `rule(input.starts_with('#'), "Must start with #.")`.
pub fn rule(ok: bool, message: impl Into<Message>) -> Result<(), Message> {
    if ok { Ok(()) } else { Err(message.into()) }
}

/// Override a rule's message: `required(&x).or_message("Tell us your name.")`.
pub trait OrMessage {
    fn or_message(self, message: impl Into<Message>) -> Self;
}

impl OrMessage for Result<(), Message> {
    fn or_message(self, message: impl Into<Message>) -> Self {
        self.map_err(|_| message.into())
    }
}

/// Run `rule` only when there is a value; `None` passes.
/// `e.check("bio", nullable(self.bio.as_deref(), |b| max_len(b, 500)))`.
pub fn nullable<T>(value: Option<T>, rule: impl FnOnce(T) -> Result<(), Message>) -> Result<(), Message> {
    value.map_or(Ok(()), rule)
}

// Presence and length.

/// Not empty after trimming whitespace.
pub fn required(s: &str) -> Result<(), Message> {
    check(!s.trim().is_empty(), || "This field is required.".into())
}

/// At least `min` characters.
pub fn min_len(s: &str, min: usize) -> Result<(), Message> {
    check(s.chars().count() >= min, || format!("Must be at least {min} characters.").into())
}

/// At most `max` characters.
pub fn max_len(s: &str, max: usize) -> Result<(), Message> {
    check(s.chars().count() <= max, || format!("Must be at most {max} characters.").into())
}

/// Exactly `len` characters.
pub fn exact_len(s: &str, len: usize) -> Result<(), Message> {
    check(s.chars().count() == len, || format!("Must be exactly {len} characters.").into())
}

/// At least `min` items.
pub fn min_items<T>(items: &[T], min: usize) -> Result<(), Message> {
    check(items.len() >= min, || format!("Must have at least {min} items.").into())
}

/// At most `max` items.
pub fn max_items<T>(items: &[T], max: usize) -> Result<(), Message> {
    check(items.len() <= max, || format!("Must have at most {max} items.").into())
}

// Numbers.

/// Within `min..=max`.
pub fn between<N: PartialOrd + fmt::Display>(n: &N, min: N, max: N) -> Result<(), Message> {
    check(*n >= min && *n <= max, || format!("Must be between {min} and {max}.").into())
}

/// `min` or more.
pub fn at_least<N: PartialOrd + fmt::Display>(n: &N, min: N) -> Result<(), Message> {
    check(*n >= min, || format!("Must be at least {min}.").into())
}

/// `max` or less.
pub fn at_most<N: PartialOrd + fmt::Display>(n: &N, max: N) -> Result<(), Message> {
    check(*n <= max, || format!("Must be at most {max}.").into())
}

/// Text that is a whole number, such as `-12`.
pub fn integer(s: &str) -> Result<(), Message> {
    check(s.trim().parse::<i64>().is_ok(), || "Must be a whole number.".into())
}

/// Text that is a finite number, such as `3.5`.
pub fn numeric(s: &str) -> Result<(), Message> {
    check(s.trim().parse::<f64>().is_ok_and(f64::is_finite), || "Must be a number.".into())
}

// Choices and comparisons.

/// One of `allowed`.
pub fn one_of<T: PartialEq + ?Sized>(value: &T, allowed: &[&T]) -> Result<(), Message> {
    check(allowed.contains(&value), || "Must be one of the allowed values.".into())
}

/// None of `forbidden`.
pub fn not_one_of<T: PartialEq + ?Sized>(value: &T, forbidden: &[&T]) -> Result<(), Message> {
    check(!forbidden.contains(&value), || "This value is not allowed.".into())
}

/// Equal to `other`, as for a password confirmation.
pub fn same<T: PartialEq + ?Sized>(value: &T, other: &T) -> Result<(), Message> {
    check(value == other, || "Must match.".into())
}

/// Different from `other`.
pub fn different<T: PartialEq + ?Sized>(value: &T, other: &T) -> Result<(), Message> {
    check(value != other, || "Must be different.".into())
}

/// Checked, as for accepting terms.
pub fn accepted(value: bool) -> Result<(), Message> {
    check(value, || "Must be accepted.".into())
}

// Character classes.

/// Letters only (any script).
pub fn alpha(s: &str) -> Result<(), Message> {
    check(!s.is_empty() && s.chars().all(char::is_alphabetic), || "Must contain only letters.".into())
}

/// Letters and digits only.
pub fn alpha_num(s: &str) -> Result<(), Message> {
    check(!s.is_empty() && s.chars().all(char::is_alphanumeric), || "Must contain only letters and numbers.".into())
}

/// Letters, digits, `-` and `_`, as in a slug or username.
pub fn alpha_dash(s: &str) -> Result<(), Message> {
    let ok = !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    check(ok, || "Must contain only letters, numbers, dashes and underscores.".into())
}

/// No uppercase letters.
pub fn lowercase(s: &str) -> Result<(), Message> {
    check(!s.chars().any(char::is_uppercase), || "Must be lowercase.".into())
}

/// No lowercase letters.
pub fn uppercase(s: &str) -> Result<(), Message> {
    check(!s.chars().any(char::is_lowercase), || "Must be uppercase.".into())
}

pub fn starts_with(s: &str, prefix: &str) -> Result<(), Message> {
    check(s.starts_with(prefix), || format!("Must start with {prefix}.").into())
}

pub fn ends_with(s: &str, suffix: &str) -> Result<(), Message> {
    check(s.ends_with(suffix), || format!("Must end with {suffix}.").into())
}

// Formats.

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
    check(ok, || "Must be a valid email address.".into())
}

/// An `http` or `https` URL with a host.
/// ponytail: shape only; use the `url` crate in the app for strict parsing.
pub fn url(s: &str) -> Result<(), Message> {
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://"));
    let host = rest.map(|r| r.split(['/', '?', '#']).next().unwrap_or(""));
    let ok = host.is_some_and(|h| !h.is_empty() && !h.starts_with(':')) && !s.chars().any(char::is_whitespace);
    check(ok, || "Must be a valid URL.".into())
}

/// A UUID in its 36-character hyphenated form.
pub fn uuid(s: &str) -> Result<(), Message> {
    let ok = s.len() == 36
        && s.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() });
    check(ok, || "Must be a valid UUID.".into())
}

/// An IPv4 or IPv6 address.
pub fn ip(s: &str) -> Result<(), Message> {
    check(s.parse::<IpAddr>().is_ok(), || "Must be a valid IP address.".into())
}

pub fn ipv4(s: &str) -> Result<(), Message> {
    check(s.parse::<std::net::Ipv4Addr>().is_ok(), || "Must be a valid IPv4 address.".into())
}

pub fn ipv6(s: &str) -> Result<(), Message> {
    check(s.parse::<std::net::Ipv6Addr>().is_ok(), || "Must be a valid IPv6 address.".into())
}

/// A calendar date as `YYYY-MM-DD`, leap years included.
pub fn date(s: &str) -> Result<(), Message> {
    let parts: Vec<&str> = s.split('-').collect();
    let ok = match parts.as_slice() {
        [y, m, d] if y.len() == 4 && m.len() == 2 && d.len() == 2 => {
            match (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>()) {
                (Ok(y), Ok(m), Ok(d)) => {
                    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
                    let days = match m {
                        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                        4 | 6 | 9 | 11 => 30,
                        2 if leap => 29,
                        2 => 28,
                        _ => 0,
                    };
                    d >= 1 && d <= days
                }
                _ => false,
            }
        }
        _ => false,
    };
    check(ok, || "Must be a valid date (YYYY-MM-DD).".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(r: Result<(), Message>) -> bool {
        r.is_ok()
    }

    #[test]
    fn presence_and_length() {
        assert!(!ok(required("  ")) && ok(required("x")));
        assert!(!ok(min_len("ab", 3)) && ok(min_len("abc", 3)));
        assert!(ok(max_len("héllo", 5)) && !ok(max_len("héllo!", 5)));
        assert!(ok(exact_len("héllo", 5)) && !ok(exact_len("hello!", 5)));
        assert!(ok(min_items(&[1, 2], 2)) && !ok(min_items(&[1], 2)));
        assert!(ok(max_items(&[1], 1)) && !ok(max_items(&[1, 2], 1)));
    }

    #[test]
    fn numbers() {
        assert!(ok(between(&5, 1, 10)) && !ok(between(&11, 1, 10)));
        assert!(ok(at_least(&3, 3)) && !ok(at_least(&2, 3)));
        assert!(ok(at_most(&3.5, 4.0)) && !ok(at_most(&5, 4)));
        assert!(ok(integer("-12")) && !ok(integer("1.5")) && !ok(integer("x")));
        assert!(ok(numeric("3.5")) && ok(numeric("-2")) && !ok(numeric("NaN")) && !ok(numeric("inf")) && !ok(numeric("")));
    }

    #[test]
    fn choices_and_comparisons() {
        assert!(ok(one_of("b", &["a", "b"])) && !ok(one_of("c", &["a", "b"])));
        assert!(ok(not_one_of("c", &["a", "b"])) && !ok(not_one_of("a", &["a", "b"])));
        assert!(ok(same("pw", "pw")) && !ok(same("pw", "px")));
        assert!(ok(different(&1, &2)) && !ok(different(&1, &1)));
        assert!(ok(accepted(true)) && !ok(accepted(false)));
    }

    #[test]
    fn character_classes() {
        assert!(ok(alpha("héllo")) && !ok(alpha("he11o")) && !ok(alpha("")));
        assert!(ok(alpha_num("abc123")) && !ok(alpha_num("abc-123")));
        assert!(ok(alpha_dash("my-repo_2")) && !ok(alpha_dash("my repo")));
        assert!(ok(lowercase("abc-1")) && !ok(lowercase("aBc")));
        assert!(ok(uppercase("ABC-1")) && !ok(uppercase("AbC")));
        assert!(ok(starts_with("#tag", "#")) && !ok(starts_with("tag", "#")));
        assert!(ok(ends_with("a.rs", ".rs")) && !ok(ends_with("a.ts", ".rs")));
    }

    #[test]
    fn formats() {
        for good in ["a@b.co", "x.y+z@mail.example.org"] {
            assert!(ok(email(good)), "{good}");
        }
        for bad in ["", "a", "@b.co", "a@b", "a@b.", "a@@b.co", "a b@c.de"] {
            assert!(!ok(email(bad)), "{bad}");
        }
        for good in ["https://example.com", "http://localhost:3000/a?b=1", "https://x.io#top"] {
            assert!(ok(url(good)), "{good}");
        }
        for bad in ["example.com", "ftp://x.com", "https://", "https://:80", "https://a b.com"] {
            assert!(!ok(url(bad)), "{bad}");
        }
        assert!(ok(uuid("123e4567-e89b-12d3-a456-426614174000")) && !ok(uuid("123e4567e89b12d3a456426614174000")));
        assert!(!ok(uuid("123e4567-e89b-12d3-a456-42661417400g")));
        assert!(ok(ip("10.0.0.1")) && ok(ip("::1")) && !ok(ip("10.0.0.256")));
        assert!(ok(ipv4("10.0.0.1")) && !ok(ipv4("::1")) && ok(ipv6("::1")) && !ok(ipv6("10.0.0.1")));
        for good in ["2024-02-29", "2000-02-29", "1999-12-31"] {
            assert!(ok(date(good)), "{good}");
        }
        for bad in ["2023-02-29", "1900-02-29", "2024-13-01", "2024-04-31", "2024-4-01", "24-04-01", "2024-04-00"] {
            assert!(!ok(date(bad)), "{bad}");
        }
    }

    #[test]
    fn helpers() {
        assert!(ok(nullable(None::<&str>, required)) && !ok(nullable(Some(""), required)));
        assert_eq!(required("").or_message("Tell us your name."), Err("Tell us your name.".into()));
        assert!(ok(required("x").or_message("unused")));
        assert_eq!(rule(false, "Nope."), Err("Nope.".into()));
    }
}
