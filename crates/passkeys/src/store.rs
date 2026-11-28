//! Persistence: the `Store` trait and the in-memory store.

use std::{
    future::Future,
    sync::{Mutex, MutexGuard, PoisonError},
};

use db::Uuid;
use serde::Serialize;

use crate::{Credential, Error};

/// A stored passkey. Serializes (for pages) without its internals.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Passkey {
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub id: i64,
    /// Names the passkey in URLs and JSON.
    pub public_id: Uuid<Passkey>,
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub user_id: i64,
    pub name: String,
    /// The WebAuthn credential id, unique across all users.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub credential_id: Vec<u8>,
    /// The user's WebAuthn user handle: the same for all their passkeys.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub user_handle: uuid::Uuid,
    /// The public key, counter and flags (webauthn-rs `Passkey`, as JSON in SQL).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub credential: Credential,
    /// Unix seconds.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_used_at: Option<i64>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_at: i64,
}

/// What a new passkey row holds; the store assigns `id`.
#[derive(Clone, Debug)]
pub struct NewPasskey {
    pub public_id: Uuid<Passkey>,
    pub user_id: i64,
    pub name: String,
    pub credential_id: Vec<u8>,
    pub user_handle: uuid::Uuid,
    pub credential: Credential,
    pub created_at: i64,
}

/// Persistence for passkeys. The crate ships [`MemoryStore`] and, behind
/// features, SQLite, Postgres and MySQL stores. Stores keep data;
/// [`Passkeys`](crate::Passkeys) applies the rules.
pub trait Store: Send + Sync + 'static {
    fn add(&self, passkey: NewPasskey) -> impl Future<Output = Result<(), Error>> + Send;
    /// The user's passkeys, newest first.
    fn passkeys(&self, user: i64) -> impl Future<Output = Result<Vec<Passkey>, Error>> + Send;
    fn by_credential_id(&self, credential_id: &[u8]) -> impl Future<Output = Result<Option<Passkey>, Error>> + Send;
    /// Save the credential's new counter and flags and set `last_used_at`.
    fn used(&self, id: i64, credential: &Credential, now: i64) -> impl Future<Output = Result<(), Error>> + Send;
    /// `false` if the user has no passkey with this id.
    fn rename(&self, user: i64, passkey: Uuid<Passkey>, name: &str) -> impl Future<Output = Result<bool, Error>> + Send;
    /// `false` if the user has no passkey with this id.
    fn delete(&self, user: i64, passkey: Uuid<Passkey>) -> impl Future<Output = Result<bool, Error>> + Send;
    fn delete_all(&self, user: i64) -> impl Future<Output = Result<(), Error>> + Send;
}

/// Everything in memory, for tests and prototypes. Lost on restart.
#[derive(Default)]
pub struct MemoryStore(Mutex<(i64, Vec<Passkey>)>);

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Each critical section is a few plain updates, so poisoned data is still consistent.
    fn lock(&self) -> MutexGuard<'_, (i64, Vec<Passkey>)> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Store for MemoryStore {
    async fn add(&self, p: NewPasskey) -> Result<(), Error> {
        let mut s = self.lock();
        if s.1.iter().any(|k| k.credential_id == p.credential_id) {
            return Err(Error::AlreadyRegistered);
        }
        s.0 += 1;
        let id = s.0;
        s.1.push(Passkey {
            id,
            public_id: p.public_id,
            user_id: p.user_id,
            name: p.name,
            credential_id: p.credential_id,
            user_handle: p.user_handle,
            credential: p.credential,
            last_used_at: None,
            created_at: p.created_at,
        });
        Ok(())
    }

    async fn passkeys(&self, user: i64) -> Result<Vec<Passkey>, Error> {
        Ok(self.lock().1.iter().rev().filter(|p| p.user_id == user).cloned().collect())
    }

    async fn by_credential_id(&self, credential_id: &[u8]) -> Result<Option<Passkey>, Error> {
        Ok(self.lock().1.iter().find(|p| p.credential_id == credential_id).cloned())
    }

    async fn used(&self, id: i64, credential: &Credential, now: i64) -> Result<(), Error> {
        if let Some(p) = self.lock().1.iter_mut().find(|p| p.id == id) {
            p.credential = credential.clone();
            p.last_used_at = Some(now);
        }
        Ok(())
    }

    async fn rename(&self, user: i64, passkey: Uuid<Passkey>, name: &str) -> Result<bool, Error> {
        Ok(match self.lock().1.iter_mut().find(|p| p.user_id == user && p.public_id == passkey) {
            Some(p) => {
                p.name = name.to_owned();
                true
            }
            None => false,
        })
    }

    async fn delete(&self, user: i64, passkey: Uuid<Passkey>) -> Result<bool, Error> {
        let mut s = self.lock();
        let before = s.1.len();
        s.1.retain(|p| !(p.user_id == user && p.public_id == passkey));
        Ok(s.1.len() < before)
    }

    async fn delete_all(&self, user: i64) -> Result<(), Error> {
        self.lock().1.retain(|p| p.user_id != user);
        Ok(())
    }
}
