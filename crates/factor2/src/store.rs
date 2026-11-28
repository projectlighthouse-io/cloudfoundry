//! Persistence: the `Store` trait and the in-memory store.

use std::{
    future::Future,
    sync::{Mutex, MutexGuard, PoisonError},
};

use db::Uuid;
use serde::Serialize;

use crate::Error;

/// A user's TOTP secret as stored: sealed with `crypt`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSecret {
    pub sealed: Vec<u8>,
    /// Unix seconds; `None` until the user entered a first valid code.
    pub confirmed_at: Option<i64>,
    /// The last time step a code was accepted for; older or equal steps are replays.
    pub last_step: Option<i64>,
}

/// A browser that skips the challenge. Serializes without its internal ids.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TrustedDevice {
    #[serde(skip)]
    pub id: i64,
    pub uuid: Uuid<TrustedDevice>,
    #[serde(skip)]
    pub user_id: i64,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    /// Unix seconds.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub expires_at: i64,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_used_at: Option<i64>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_at: i64,
}

/// What a new trusted device row holds; the store assigns `id`.
#[derive(Clone, Debug)]
pub struct NewDevice {
    pub uuid: Uuid<TrustedDevice>,
    pub user_id: i64,
    pub token_hash: Vec<u8>,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub expires_at: i64,
    pub created_at: i64,
}

/// Persistence for secrets, backup codes and trusted devices. The crate ships
/// [`MemoryStore`] and, behind features, SQLite, Postgres and MySQL stores.
/// Stores keep data; [`TwoFactor`](crate::TwoFactor) applies the rules.
pub trait Store: Send + Sync + 'static {
    fn secret(&self, user: i64) -> impl Future<Output = Result<Option<StoredSecret>, Error>> + Send;
    /// Replace the user's secret with a new, unconfirmed one.
    fn put_secret(&self, user: i64, sealed: &[u8], now: i64) -> impl Future<Output = Result<(), Error>> + Send;
    /// Confirm an unconfirmed secret, recording `step` as used. `false` if
    /// there is no unconfirmed secret.
    fn confirm_secret(&self, user: i64, now: i64, step: i64) -> impl Future<Output = Result<bool, Error>> + Send;
    /// Record `step` as used, only if it is newer than the last one, in one
    /// statement. `false` means it was already used (a replay or a race).
    fn use_step(&self, user: i64, step: i64) -> impl Future<Output = Result<bool, Error>> + Send;
    fn delete_secret(&self, user: i64) -> impl Future<Output = Result<(), Error>> + Send;

    /// Replace all of the user's backup codes.
    fn replace_backup_codes(&self, user: i64, hashes: &[Vec<u8>]) -> impl Future<Output = Result<(), Error>> + Send;
    /// Mark the unused code with this hash used, in one statement. `false` if
    /// there is none (wrong or already used).
    fn use_backup_code(&self, user: i64, hash: &[u8], now: i64) -> impl Future<Output = Result<bool, Error>> + Send;
    fn unused_backup_codes(&self, user: i64) -> impl Future<Output = Result<usize, Error>> + Send;

    fn add_device(&self, device: NewDevice) -> impl Future<Output = Result<(), Error>> + Send;
    fn device_by_token(&self, token_hash: &[u8]) -> impl Future<Output = Result<Option<TrustedDevice>, Error>> + Send;
    fn touch_device(&self, id: i64, now: i64) -> impl Future<Output = Result<(), Error>> + Send;
    /// The user's devices, newest first.
    fn devices(&self, user: i64) -> impl Future<Output = Result<Vec<TrustedDevice>, Error>> + Send;
    /// `false` if the user has no device with this uuid.
    fn delete_device(&self, user: i64, uuid: Uuid<TrustedDevice>) -> impl Future<Output = Result<bool, Error>> + Send;

    /// Remove everything for the user: secret, backup codes and devices.
    fn delete_all(&self, user: i64) -> impl Future<Output = Result<(), Error>> + Send;
}

/// Everything in memory, for tests and prototypes. Lost on restart.
#[derive(Default)]
pub struct MemoryStore(Mutex<State>);

#[derive(Default)]
struct State {
    next: i64,
    secrets: Vec<(i64, StoredSecret)>,
    /// (user, hash, used)
    codes: Vec<(i64, Vec<u8>, bool)>,
    devices: Vec<(TrustedDevice, Vec<u8>)>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Each critical section is a few plain updates, so poisoned data is still consistent.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl State {
    fn secret_mut(&mut self, user: i64) -> Option<&mut StoredSecret> {
        self.secrets.iter_mut().find(|(u, _)| *u == user).map(|(_, s)| s)
    }
}

impl Store for MemoryStore {
    async fn secret(&self, user: i64) -> Result<Option<StoredSecret>, Error> {
        Ok(self.lock().secrets.iter().find(|(u, _)| *u == user).map(|(_, s)| s.clone()))
    }

    async fn put_secret(&self, user: i64, sealed: &[u8], _now: i64) -> Result<(), Error> {
        let mut s = self.lock();
        s.secrets.retain(|(u, _)| *u != user);
        s.secrets.push((user, StoredSecret { sealed: sealed.to_vec(), confirmed_at: None, last_step: None }));
        Ok(())
    }

    async fn confirm_secret(&self, user: i64, now: i64, step: i64) -> Result<bool, Error> {
        let mut s = self.lock();
        Ok(match s.secret_mut(user) {
            Some(secret) if secret.confirmed_at.is_none() => {
                secret.confirmed_at = Some(now);
                secret.last_step = Some(step);
                true
            }
            _ => false,
        })
    }

    async fn use_step(&self, user: i64, step: i64) -> Result<bool, Error> {
        let mut s = self.lock();
        Ok(match s.secret_mut(user) {
            Some(secret) if secret.confirmed_at.is_some() && secret.last_step.is_none_or(|l| l < step) => {
                secret.last_step = Some(step);
                true
            }
            _ => false,
        })
    }

    async fn delete_secret(&self, user: i64) -> Result<(), Error> {
        self.lock().secrets.retain(|(u, _)| *u != user);
        Ok(())
    }

    async fn replace_backup_codes(&self, user: i64, hashes: &[Vec<u8>]) -> Result<(), Error> {
        let mut s = self.lock();
        s.codes.retain(|(u, ..)| *u != user);
        s.codes.extend(hashes.iter().map(|h| (user, h.clone(), false)));
        Ok(())
    }

    async fn use_backup_code(&self, user: i64, hash: &[u8], _now: i64) -> Result<bool, Error> {
        let mut s = self.lock();
        Ok(match s.codes.iter_mut().find(|(u, h, used)| *u == user && h == hash && !used) {
            Some(code) => {
                code.2 = true;
                true
            }
            None => false,
        })
    }

    async fn unused_backup_codes(&self, user: i64) -> Result<usize, Error> {
        Ok(self.lock().codes.iter().filter(|(u, _, used)| *u == user && !used).count())
    }

    async fn add_device(&self, d: NewDevice) -> Result<(), Error> {
        let mut s = self.lock();
        s.next += 1;
        let device = TrustedDevice {
            id: s.next,
            uuid: d.uuid,
            user_id: d.user_id,
            user_agent: d.user_agent,
            ip: d.ip,
            expires_at: d.expires_at,
            last_used_at: None,
            created_at: d.created_at,
        };
        s.devices.push((device, d.token_hash));
        Ok(())
    }

    async fn device_by_token(&self, token_hash: &[u8]) -> Result<Option<TrustedDevice>, Error> {
        Ok(self.lock().devices.iter().find(|(_, h)| h == token_hash).map(|(d, _)| d.clone()))
    }

    async fn touch_device(&self, id: i64, now: i64) -> Result<(), Error> {
        if let Some((d, _)) = self.lock().devices.iter_mut().find(|(d, _)| d.id == id) {
            d.last_used_at = Some(now);
        }
        Ok(())
    }

    async fn devices(&self, user: i64) -> Result<Vec<TrustedDevice>, Error> {
        Ok(self.lock().devices.iter().rev().filter(|(d, _)| d.user_id == user).map(|(d, _)| d.clone()).collect())
    }

    async fn delete_device(&self, user: i64, uuid: Uuid<TrustedDevice>) -> Result<bool, Error> {
        let mut s = self.lock();
        let before = s.devices.len();
        s.devices.retain(|(d, _)| !(d.user_id == user && d.uuid == uuid));
        Ok(s.devices.len() < before)
    }

    async fn delete_all(&self, user: i64) -> Result<(), Error> {
        let mut s = self.lock();
        s.secrets.retain(|(u, _)| *u != user);
        s.codes.retain(|(u, ..)| *u != user);
        s.devices.retain(|(d, _)| d.user_id != user);
        Ok(())
    }
}
