#![allow(clippy::unwrap_used)]

use std::time::Duration;

use throttle::{Algorithm, FixedWindow, TokenBucket};

/// Regression: a zero refill rate made the refill time infinite and overflowed `now + ..`.
#[test]
fn zero_refill_bucket_does_not_overflow() {
    let b = TokenBucket::new(1, 0, Duration::from_secs(1));
    let (first, state) = b.decide(None, 1_000, 1);
    assert!(first.allowed);
    let (second, _) = b.decide(state.as_ref().map(|(s, _)| s.as_slice()), 1_001, 1);
    assert!(!second.allowed);
}

#[cfg(feature = "configured")]
mod configured_tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Limits {
        login: FixedWindow,
        api: TokenBucket,
    }

    #[test]
    fn limits_load_and_behave() {
        let l: Limits = toml::from_str("[login]\nmax = 2\nwindow = \"1m\"\n[api]\ncapacity = 3\nrefill = 1\nevery = \"1s\"\n").unwrap();
        let (a, s) = l.login.decide(None, 0, 1);
        let (b, s) = l.login.decide(s.as_ref().map(|(s, _)| s.as_slice()), 1, 1);
        let (c, _) = l.login.decide(s.as_ref().map(|(s, _)| s.as_slice()), 2, 1);
        assert!(a.allowed && b.allowed && !c.allowed);
        assert_eq!(c.retry_after, Duration::from_millis(59_998));
        assert!(l.api.decide(None, 0, 3).0.allowed);
    }

    #[test]
    fn rejects_limits_that_deny_everything_or_never_refill() {
        let window = |t: &str| toml::from_str::<FixedWindow>(t);
        assert!(window("max = 0\nwindow = \"1m\"\n").is_err());
        assert!(window("max = 1\nwindow = \"0s\"\n").is_err());
        assert!(window("max = 1\nwindow = \"1m\"\nmaks = 2\n").is_err(), "unknown key");
        let bucket = |t: &str| toml::from_str::<TokenBucket>(t);
        assert!(bucket("capacity = 1\nrefill = 0\nevery = \"1s\"\n").is_err());
        assert!(bucket("capacity = 0\nrefill = 1\nevery = \"1s\"\n").is_err());
    }
}
