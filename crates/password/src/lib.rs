//! Hash and verify passwords with Argon2id (OWASP-recommended defaults).
//!
//! ```
//! let hash = password::hash("hunter2").unwrap();
//! assert!(password::verify("hunter2", &hash).is_ok());
//! assert_eq!(password::verify("wrong", &hash), Err(password::Error::Mismatch));
//! ```
//!
//! Both calls take tens of milliseconds of CPU by design. In an async handler
//! under load, run them in `tokio::task::spawn_blocking`.

use std::{fmt, sync::OnceLock};

use argon2::{
    Argon2,
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

/// Spend the same time as `verify` when there is no user to check against,
/// so response time does not reveal which accounts exist.
pub fn verify_none(password: &str) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let dummy = DUMMY.get_or_init(|| hash("dummy password").expect("hashing a constant"));
    let _ = verify(password, dummy);
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_malformed_hash() {
        assert_eq!(super::verify("x", "not a hash"), Err(super::Error::Hash));
        super::verify_none("x");
    }
}
