//! Hash and verify passwords with Argon2id (OWASP-recommended defaults).
//!
//! ```
//! let hash = password::hash("hunter2").unwrap();
//! assert!(password::verify("hunter2", &hash).is_ok());
//! assert_eq!(password::verify("wrong", &hash), Err(password::Error::Mismatch));
//! assert!(!password::needs_rehash(&hash));
//! ```
//!
//! Both calls take tens of milliseconds of CPU by design. In an async handler
//! under load, run them in `tokio::task::spawn_blocking`.

use std::{fmt, sync::OnceLock};

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The password does not match.
    Mismatch,
    /// The stored hash is malformed, or hashing failed.
    Hash,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::Mismatch => "password does not match",
            Error::Hash => "invalid password hash",
        })
    }
}

impl std::error::Error for Error {}

/// A PHC string (`$argon2id$v=19$...`) with a random salt, ready to store.
pub fn hash(password: &str) -> Result<String, Error> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| Error::Hash)?;
    let salt = SaltString::encode_b64(&salt).map_err(|_| Error::Hash)?;
    Ok(Argon2::default().hash_password(password.as_bytes(), &salt).map_err(|_| Error::Hash)?.to_string())
}

/// Check `password` against a stored hash. Parameters come from the hash, so
/// old hashes keep verifying after the defaults change.
pub fn verify(password: &str, hash: &str) -> Result<(), Error> {
    let parsed = PasswordHash::new(hash).map_err(|_| Error::Hash)?;
    Argon2::default().verify_password(password.as_bytes(), &parsed).map_err(|e| match e {
        argon2::password_hash::Error::Password => Error::Mismatch,
        _ => Error::Hash,
    })
}

/// Whether `hash` was made with other settings than [`hash`] uses today
/// (older parameters, another algorithm, or unreadable). Check after a
/// successful `verify` and store a fresh `hash(password)` when true.
pub fn needs_rehash(hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else { return true };
    let Ok(params) = Params::try_from(&parsed) else { return true };
    let current = Params::default();
    parsed.algorithm != Algorithm::Argon2id.ident()
        || parsed.version != Some(Version::V0x13.into())
        || (params.m_cost(), params.t_cost(), params.p_cost()) != (current.m_cost(), current.t_cost(), current.p_cost())
}

/// Spend the same time as `verify` when there is no user to check against,
/// so response time does not reveal which accounts exist.
pub fn verify_none(password: &str) {
    static DUMMY: OnceLock<Option<String>> = OnceLock::new();
    // Hashing a constant only fails without OS randomness; then skip the padding rather than panic.
    if let Some(dummy) = DUMMY.get_or_init(|| hash("dummy password").ok()) {
        let _ = verify(password, dummy);
    }
}

#[cfg(test)]
mod tests {
    use argon2::PasswordHasher;
    #[test]
    fn rejects_malformed_hash() {
        assert_eq!(super::verify("x", "not a hash"), Err(super::Error::Hash));
        super::verify_none("x");
    }

    #[test]
    fn needs_rehash_only_for_other_settings() {
        assert!(!super::needs_rehash(&super::hash("x").unwrap()));
        let weak = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, argon2::Params::new(8192, 1, 1, None).unwrap());
        let salt = argon2::password_hash::SaltString::encode_b64(b"0123456789abcdef").unwrap();
        let old = weak.hash_password(b"x", &salt).unwrap().to_string();
        assert!(super::needs_rehash(&old), "older parameters");
        assert!(super::verify("x", &old).is_ok(), "still verifies");
        assert!(super::needs_rehash("not a hash"));
    }
}
