//! Encrypt values at rest (database columns, files), with key rotation.
//!
//! ```
//! use crypt::{Encrypter, Keys};
//!
//! let crypt = Encrypter::new(Keys::parse(&Keys::generate()).unwrap());
//! let sealed = crypt.encrypt(b"totp:42", b"secret").unwrap();
//! assert_eq!(crypt.decrypt(b"totp:42", &sealed).unwrap(), b"secret");
//! assert!(crypt.decrypt(b"totp:43", &sealed).is_err(), "bound to its purpose");
//! ```

use std::fmt;

use base64::{Engine, engine::general_purpose::STANDARD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

const VERSION: u8 = 1;
const ID: usize = 4;
const NONCE: usize = 24;
const HEADER: usize = 1 + ID + NONCE;

/// One 32-byte key. Never printed: no `Debug`, no `Display`.
pub struct Key([u8; 32]);

/// Keys, newest first. Encrypting uses the newest; decrypting finds the key a
/// value was sealed with, so old keys can stay while values are re-encrypted.
pub struct Keys(Vec<Key>);

impl Keys {
    /// Comma-separated base64 keys, newest first. A `base64:` prefix is allowed.
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        let keys = s
            .split(',')
            .map(|k| {
                let bytes = STANDARD.decode(k.trim().trim_start_matches("base64:")).map_err(|_| KeyError::Format)?;
                Ok(Key(bytes.try_into().map_err(|_| KeyError::Length)?))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if keys.is_empty() {
            return Err(KeyError::Format);
        }
        Ok(Keys(keys))
    }

    /// `Keys::parse` on an environment variable.
    pub fn from_env(var: &str) -> Result<Self, KeyError> {
        Self::parse(&std::env::var(var).map_err(|_| KeyError::Missing(var.to_owned()))?)
    }

    /// A fresh random key, base64, for `APP_KEY`.
    pub fn generate() -> String {
        STANDARD.encode(random::<32>())
    }
}

#[derive(Debug)]
pub enum KeyError {
    Missing(String),
    Format,
    Length,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(v) => write!(f, "{v} is not set"),
            Self::Format => f.write_str("key is not valid base64"),
            Self::Length => f.write_str("key must be 32 bytes"),
        }
    }
}

impl std::error::Error for KeyError {}

/// Every failure. Deliberately says nothing about why.
#[derive(Debug, PartialEq, Eq)]
pub struct Error;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cannot encrypt or decrypt this value")
    }
}

impl std::error::Error for Error {}

struct Derived {
    /// Names the key in sealed values without revealing it: the first bytes
    /// of an HMAC under the key.
    id: [u8; ID],
    aead: XChaCha20Poly1305,
}

type HmacSha256 = Hmac<Sha256>;

#[allow(clippy::expect_used, reason = "HMAC accepts keys of any length, so this cannot fail")]
fn derive(key: &Key, label: &[u8]) -> [u8; 32] {
    let mut m = <HmacSha256 as Mac>::new_from_slice(&key.0).expect("any key length");
    m.update(label);
    m.finalize().into_bytes().into()
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    // No secure fallback exists: a predictable nonce or key must not be used.
    #[allow(clippy::expect_used, reason = "the OS random source failed; there is no safe fallback")]
    getrandom::fill(&mut b).expect("OS random source");
    b
}

/// Seals and opens values. Cheap to share behind an `Arc`.
pub struct Encrypter {
    keys: Vec<Derived>,
}

impl Encrypter {
    pub fn new(keys: Keys) -> Self {
        let keys = keys
            .0
            .iter()
            .map(|k| {
                let id = derive(k, b"crypt/id");
                Derived { id: [id[0], id[1], id[2], id[3]], aead: XChaCha20Poly1305::new(&derive(k, b"crypt/encrypt").into()) }
            })
            .collect();
        Encrypter { keys }
    }

    fn newest(&self) -> Result<&Derived, Error> {
        self.keys.first().ok_or(Error)
    }

    /// Seal `plain` for `purpose` with the newest key.
    pub fn encrypt(&self, purpose: &[u8], plain: &[u8]) -> Result<Vec<u8>, Error> {
        let key = self.newest()?;
        let nonce = random::<NONCE>();
        let sealed = key.aead.encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: purpose }).map_err(|_| Error)?;
        let mut out = Vec::with_capacity(HEADER + sealed.len());
        out.push(VERSION);
        out.extend_from_slice(&key.id);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// Open a value from [`encrypt`](Self::encrypt) with the same `purpose`.
    pub fn decrypt(&self, purpose: &[u8], sealed: &[u8]) -> Result<Vec<u8>, Error> {
        let (id, nonce, body) = split(sealed)?;
        let key = self.keys.iter().find(|k| k.id == id).ok_or(Error)?;
        key.aead.decrypt(XNonce::from_slice(nonce), Payload { msg: body, aad: purpose }).map_err(|_| Error)
    }

    /// Whether `sealed` was made with an older key: decrypt and encrypt it
    /// again to move it to the newest one before dropping the old key.
    pub fn needs_rotation(&self, sealed: &[u8]) -> bool {
        match (split(sealed), self.newest()) {
            (Ok((id, ..)), Ok(newest)) => id != newest.id,
            _ => true,
        }
    }
}

/// Key id, nonce, ciphertext.
type Parts<'a> = ([u8; ID], &'a [u8], &'a [u8]);

fn split(sealed: &[u8]) -> Result<Parts<'_>, Error> {
    if sealed.len() < HEADER || sealed[0] != VERSION {
        return Err(Error);
    }
    let id = [sealed[1], sealed[2], sealed[3], sealed[4]];
    Ok((id, &sealed[1 + ID..HEADER], &sealed[HEADER..]))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn crypt() -> Encrypter {
        Encrypter::new(Keys::parse(&Keys::generate()).unwrap())
    }

    #[test]
    fn round_trips_and_binds_purpose() {
        let c = crypt();
        let sealed = c.encrypt(b"a", b"hello").unwrap();
        assert_eq!(c.decrypt(b"a", &sealed).unwrap(), b"hello");
        assert_eq!(c.decrypt(b"b", &sealed), Err(Error));
        assert_ne!(c.encrypt(b"a", b"hello").unwrap(), sealed, "fresh nonce each time");
    }

    #[test]
    fn rejects_other_keys_tampering_and_garbage() {
        let c = crypt();
        let mut sealed = c.encrypt(b"a", b"hello").unwrap();
        assert_eq!(crypt().decrypt(b"a", &sealed), Err(Error), "other key");
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert_eq!(c.decrypt(b"a", &sealed), Err(Error), "tampered");
        assert_eq!(c.decrypt(b"a", b""), Err(Error));
        assert_eq!(c.decrypt(b"a", &[2; 64]), Err(Error), "unknown version");
    }

    #[test]
    fn rotates_keys() {
        let (old, new) = (Keys::generate(), Keys::generate());
        let before = Encrypter::new(Keys::parse(&old).unwrap()).encrypt(b"a", b"x").unwrap();
        let both = Encrypter::new(Keys::parse(&format!("{new},{old}")).unwrap());
        assert_eq!(both.decrypt(b"a", &before).unwrap(), b"x", "old key still opens");
        assert!(both.needs_rotation(&before));
        assert!(!both.needs_rotation(&both.encrypt(b"a", b"x").unwrap()));
        assert_eq!(Encrypter::new(Keys::parse(&new).unwrap()).decrypt(b"a", &before), Err(Error), "dropped key");
    }

    #[test]
    fn keys_must_be_32_bytes_of_base64() {
        assert!(matches!(Keys::parse("nope!"), Err(KeyError::Format)));
        assert!(matches!(Keys::parse(&STANDARD.encode([0u8; 16])), Err(KeyError::Length)));
        assert!(Keys::parse(&format!("base64:{}", Keys::generate())).is_ok());
    }
}
