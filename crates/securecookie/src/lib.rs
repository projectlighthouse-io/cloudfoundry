//! Sign and encrypt cookie values.
//!
//! ```
//! use std::time::Duration;
//! use securecookie::{Codec, Keys};
//!
//! let codec = Codec::new(Keys::parse(&Keys::generate()).unwrap());
//! let value = codec.encrypt("session", &"abc").unwrap();
//! let back: String = codec.decrypt("session", &value, Duration::from_secs(60)).unwrap();
//! assert_eq!(back, "abc");
//! assert!(codec.decrypt::<String>("other", &value, Duration::from_secs(60)).is_err());
//! ```
//!
//! Pure functions over strings: no cookie headers, no storage.

use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use hmac::{Hmac, Mac};
use serde::{Serialize, de::DeserializeOwned};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const NONCE: usize = 24;
const TAG: usize = 32;
const TS: usize = 8;
/// Clock skew tolerated for timestamps from the future.
const SKEW: u64 = 60;

/// One 32-byte key. Never printed: no `Debug`, no `Display`.
pub struct Key([u8; 32]);

/// Keys, newest first. Encoding uses the newest; decoding tries each, so an
/// old key can be kept while values written with it expire.
pub struct Keys(Vec<Key>);

impl Keys {
    /// Comma-separated base64 keys, newest first.
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        let keys = s
            .split(',')
            .map(|k| {
                let bytes = STANDARD.decode(k.trim().trim_start_matches("base64:")).map_err(|_| KeyError::Format)?;
                Ok(Key(bytes.try_into().map_err(|_| KeyError::Length)?))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Keys(keys))
    }

    /// `Keys::parse` on an environment variable.
    pub fn from_env(var: &str) -> Result<Self, KeyError> {
        Self::parse(&std::env::var(var).map_err(|_| KeyError::Missing(var.to_owned()))?)
    }

    /// A fresh random key, base64, for `APP_KEY`.
    pub fn generate() -> String {
        let mut k = [0u8; 32];
        getrandom::fill(&mut k).expect("OS random source");
        STANDARD.encode(k)
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

/// Every decode failure. Deliberately says nothing about why.
#[derive(Debug, PartialEq, Eq)]
pub struct Error;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid or expired value")
    }
}

impl std::error::Error for Error {}

struct Derived {
    aead: XChaCha20Poly1305,
    mac: [u8; 32],
}

/// Separate subkeys for encryption and signing, so one key never serves both.
fn derive(key: &Key, label: &[u8]) -> [u8; 32] {
    let mut m = <HmacSha256 as Mac>::new_from_slice(&key.0).expect("any key length");
    m.update(label);
    m.finalize().into_bytes().into()
}

pub struct Codec {
    keys: Vec<Derived>,
}

impl Codec {
    /// Panics on an empty key list: that is a startup bug.
    pub fn new(keys: Keys) -> Self {
        assert!(!keys.0.is_empty(), "securecookie needs at least one key");
        let keys = keys
            .0
            .iter()
            .map(|k| Derived {
                aead: XChaCha20Poly1305::new(&derive(k, b"securecookie/encrypt").into()),
                mac: derive(k, b"securecookie/sign"),
            })
            .collect();
        Codec { keys }
    }

    /// Encrypt and authenticate `value` for cookie `name`.
    pub fn encrypt<T: Serialize + ?Sized>(&self, name: &str, value: &T) -> Result<String, Error> {
        let plain = stamp(value)?;
        let mut nonce = [0u8; NONCE];
        getrandom::fill(&mut nonce).map_err(|_| Error)?;
        let ct = self.keys[0]
            .aead
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: &plain, aad: name.as_bytes() })
            .map_err(|_| Error)?;
        Ok(URL_SAFE_NO_PAD.encode([&nonce[..], &ct].concat()))
    }

    /// Decrypt a value from `encrypt`, rejecting it if older than `max_age`.
    pub fn decrypt<T: DeserializeOwned>(&self, name: &str, value: &str, max_age: Duration) -> Result<T, Error> {
        let raw = URL_SAFE_NO_PAD.decode(value).map_err(|_| Error)?;
        if raw.len() < NONCE {
            return Err(Error);
        }
        let (nonce, ct) = raw.split_at(NONCE);
        let plain = self
            .keys
            .iter()
            .find_map(|k| k.aead.decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: name.as_bytes() }).ok())
            .ok_or(Error)?;
        unstamp(&plain, max_age)
    }

    /// Sign `value` for cookie `name`. The value is readable, not secret.
    pub fn sign<T: Serialize + ?Sized>(&self, name: &str, value: &T) -> Result<String, Error> {
        let payload = stamp(value)?;
        let tag = mac(&self.keys[0].mac, name, &payload).finalize().into_bytes();
        Ok(URL_SAFE_NO_PAD.encode([&payload[..], &tag].concat()))
    }

    /// Verify a value from `sign`, rejecting it if older than `max_age`.
    pub fn verify<T: DeserializeOwned>(&self, name: &str, value: &str, max_age: Duration) -> Result<T, Error> {
        let raw = URL_SAFE_NO_PAD.decode(value).map_err(|_| Error)?;
        if raw.len() < TAG + TS {
            return Err(Error);
        }
        let (payload, tag) = raw.split_at(raw.len() - TAG);
        // `verify_slice` compares in constant time.
        if !self.keys.iter().any(|k| mac(&k.mac, name, payload).verify_slice(tag).is_ok()) {
            return Err(Error);
        }
        unstamp(payload, max_age)
    }
}

/// The name is bound into the tag, so a value cannot be replayed under another name.
fn mac(key: &[u8; 32], name: &str, payload: &[u8]) -> HmacSha256 {
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("any key length");
    m.update(&(name.len() as u32).to_be_bytes());
    m.update(name.as_bytes());
    m.update(payload);
    m
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn stamp<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, Error> {
    let mut out = now().to_be_bytes().to_vec();
    serde_json::to_writer(&mut out, value).map_err(|_| Error)?;
    Ok(out)
}

fn unstamp<T: DeserializeOwned>(raw: &[u8], max_age: Duration) -> Result<T, Error> {
    let (ts, json) = raw.split_at_checked(TS).ok_or(Error)?;
    let ts = u64::from_be_bytes(ts.try_into().map_err(|_| Error)?);
    let now = now();
    if ts > now + SKEW || now.saturating_sub(ts) > max_age.as_secs() {
        return Err(Error);
    }
    serde_json::from_slice(json).map_err(|_| Error)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);

    fn codec() -> Codec {
        Codec::new(Keys::parse(&Keys::generate()).unwrap())
    }

    #[test]
    fn round_trips_and_rejects_tampering() {
        let c = codec();
        let enc = c.encrypt("s", &42u32).unwrap();
        assert_eq!(c.decrypt::<u32>("s", &enc, HOUR), Ok(42));
        assert_eq!(c.decrypt::<u32>("t", &enc, HOUR), Err(Error), "bound to name");
        assert_eq!(codec().decrypt::<u32>("s", &enc, HOUR), Err(Error), "other key");

        let signed = c.sign("r", "tok").unwrap();
        assert_eq!(c.verify::<String>("r", &signed, HOUR).unwrap(), "tok");
        assert_eq!(c.verify::<String>("x", &signed, HOUR), Err(Error), "bound to name");
        let mut raw = URL_SAFE_NO_PAD.decode(&signed).unwrap();
        raw[TS] ^= 1;
        assert_eq!(c.verify::<String>("r", &URL_SAFE_NO_PAD.encode(raw), HOUR), Err(Error), "tampered");
        assert_eq!(c.decrypt::<u32>("s", "!!", HOUR), Err(Error));
    }

    #[test]
    fn max_age() {
        let c = codec();
        let mut plain = (now() - 120).to_be_bytes().to_vec();
        plain.extend(b"1");
        assert_eq!(unstamp::<u32>(&plain, Duration::from_secs(60)), Err(Error));
        assert_eq!(unstamp::<u32>(&plain, Duration::from_secs(600)), Ok(1));
        let fresh = c.encrypt("s", &1).unwrap();
        assert_eq!(c.decrypt::<u32>("s", &fresh, Duration::ZERO), Ok(1));
    }

    #[test]
    fn rotation() {
        let (old, new) = (Keys::generate(), Keys::generate());
        let before = Codec::new(Keys::parse(&old).unwrap()).encrypt("s", &7).unwrap();
        let after = Codec::new(Keys::parse(&format!("{new},{old}")).unwrap());
        assert_eq!(after.decrypt::<i32>("s", &before, HOUR), Ok(7));
        let fresh = after.encrypt("s", &8).unwrap();
        assert_eq!(Codec::new(Keys::parse(&new).unwrap()).decrypt::<i32>("s", &fresh, HOUR), Ok(8), "newest encodes");
    }
}
