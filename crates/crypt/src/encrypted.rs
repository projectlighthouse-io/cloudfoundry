//! `Encrypted<T>`: a serde value sealed for a database column.

use std::{fmt, marker::PhantomData};

use serde::{Serialize, de::DeserializeOwned};

use crate::{Encrypter, Error};

/// A value of type `T`, sealed with [`Encrypter`] as JSON. Stored as bytes
/// (`bytea` / `blob` / `varbinary`) with feature `sqlx`. Opening needs the
/// encrypter and the same `purpose`, so a value copied to another row or
/// column does not open there when the purpose names it (`"users.ssn:{id}"`).
///
/// ```
/// use crypt::{Encrypted, Encrypter, Keys};
///
/// let crypt = Encrypter::new(Keys::parse(&Keys::generate()).unwrap());
/// let ssn = Encrypted::seal(&crypt, b"users.ssn:7", &"078-05-1120".to_owned()).unwrap();
/// assert_eq!(format!("{ssn:?}"), "…");
/// assert_eq!(ssn.open(&crypt, b"users.ssn:7").unwrap(), "078-05-1120");
/// assert!(ssn.open(&crypt, b"users.ssn:8").is_err());
/// ```
pub struct Encrypted<T> {
    sealed: Vec<u8>,
    value: PhantomData<fn() -> T>,
}

impl<T> Encrypted<T> {
    /// Bytes as stored, e.g. read from a column without `sqlx`.
    pub fn from_bytes(sealed: Vec<u8>) -> Self {
        Self { sealed, value: PhantomData }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.sealed
    }

    /// Sealed with an older key: re-seal it (`crypt:rotate` does, per column).
    pub fn needs_rotation(&self, crypt: &Encrypter) -> bool {
        crypt.needs_rotation(&self.sealed)
    }
}

impl<T: Serialize> Encrypted<T> {
    pub fn seal(crypt: &Encrypter, purpose: &[u8], value: &T) -> Result<Self, Error> {
        let json = serde_json::to_vec(value).map_err(|_| Error)?;
        Ok(Self::from_bytes(crypt.encrypt(purpose, &json)?))
    }
}

impl<T: DeserializeOwned> Encrypted<T> {
    pub fn open(&self, crypt: &Encrypter, purpose: &[u8]) -> Result<T, Error> {
        serde_json::from_slice(&crypt.decrypt(purpose, &self.sealed)?).map_err(|_| Error)
    }
}

impl<T> Clone for Encrypted<T> {
    fn clone(&self) -> Self {
        Self::from_bytes(self.sealed.clone())
    }
}

impl<T> PartialEq for Encrypted<T> {
    fn eq(&self, other: &Self) -> bool {
        self.sealed == other.sealed
    }
}

impl<T> Eq for Encrypted<T> {}

/// Never shows the value, nor the ciphertext.
impl<T> fmt::Debug for Encrypted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("…")
    }
}

#[cfg(feature = "sqlx")]
mod sql {
    use sqlx::{Database, Decode, Encode, Type, encode::IsNull, error::BoxDynError};

    use super::Encrypted;

    // What `#[sqlx(transparent)]` would write, without bounds on `T`.
    impl<T, DB: Database> Type<DB> for Encrypted<T>
    where
        Vec<u8>: Type<DB>,
    {
        fn type_info() -> DB::TypeInfo {
            <Vec<u8> as Type<DB>>::type_info()
        }
        fn compatible(ty: &DB::TypeInfo) -> bool {
            <Vec<u8> as Type<DB>>::compatible(ty)
        }
    }

    impl<'q, T, DB: Database> Encode<'q, DB> for Encrypted<T>
    where
        Vec<u8>: Encode<'q, DB>,
    {
        fn encode_by_ref(&self, buf: &mut <DB as Database>::ArgumentBuffer<'q>) -> Result<IsNull, BoxDynError> {
            self.sealed.encode_by_ref(buf)
        }
    }

    impl<'r, T, DB: Database> Decode<'r, DB> for Encrypted<T>
    where
        Vec<u8>: Decode<'r, DB>,
    {
        fn decode(value: <DB as Database>::ValueRef<'r>) -> Result<Self, BoxDynError> {
            Vec::<u8>::decode(value).map(Self::from_bytes)
        }
    }
}
