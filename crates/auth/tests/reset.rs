#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use auth::{Credentials, Provider, ResetError, Resets};

#[derive(Clone)]
struct User {
    id: u32,
    email: &'static str,
    hash: String,
}

#[derive(Clone)]
struct Users {
    users: Arc<Mutex<Vec<User>>>,
    resets: Arc<Mutex<HashMap<u32, (String, i64)>>>,
}

impl Provider for Users {
    type User = User;
    type Id = u32;
    async fn find(&self, id: u32) -> Result<Option<User>, auth::Error> {
        Ok(self.users.lock().unwrap().iter().find(|u| u.id == id).cloned())
    }
    fn id(&self, user: &User) -> u32 {
        user.id
    }
}

impl Credentials for Users {
    async fn by_login(&self, login: &str) -> Result<Option<User>, auth::Error> {
        Ok(self.users.lock().unwrap().iter().find(|u| u.email == login).cloned())
    }
    fn password_hash(&self, user: &User) -> Option<String> {
        Some(user.hash.clone())
    }
    async fn set_password_hash(&self, id: u32, hash: String) -> Result<(), auth::Error> {
        self.users.lock().unwrap().iter_mut().find(|u| u.id == id).unwrap().hash = hash;
        Ok(())
    }
}

impl Resets for Users {
    async fn store_reset(&self, id: u32, hash: String, created_at: i64) -> Result<(), auth::Error> {
        self.resets.lock().unwrap().insert(id, (hash, created_at));
        Ok(())
    }
    async fn find_reset(&self, id: u32) -> Result<Option<(String, i64)>, auth::Error> {
        Ok(self.resets.lock().unwrap().get(&id).cloned())
    }
    async fn delete_reset(&self, id: u32) -> Result<(), auth::Error> {
        self.resets.lock().unwrap().remove(&id);
        Ok(())
    }
}

fn users() -> Users {
    Users {
        users: Arc::new(Mutex::new(vec![User { id: 1, email: "ada@example.com", hash: password::hash("old").unwrap() }])),
        resets: Arc::default(),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

#[tokio::test(flavor = "multi_thread")]
async fn reset_with_the_emailed_token() {
    let u = users();
    assert!(matches!(auth::reset_link_token(&u, "nobody@example.com").await, Err(ResetError::UnknownUser)));

    let (user, token) = auth::reset_link_token(&u, "ada@example.com").await.unwrap();
    assert_eq!(user.id, 1);
    assert_eq!(u.resets.lock().unwrap()[&1].0, auth::hash_token(&token), "only the hash is stored");
    assert!(matches!(auth::reset_link_token(&u, "ada@example.com").await, Err(ResetError::Throttled)), "one link a minute");

    assert!(matches!(auth::reset_password(&u, "ada@example.com", "wrong", "new-password").await, Err(ResetError::InvalidToken)));
    assert!(matches!(auth::reset_password(&u, "nobody@example.com", &token, "new-password").await, Err(ResetError::InvalidToken)));

    auth::reset_password(&u, "ada@example.com", &token, "new-password").await.unwrap();
    let hash = u.users.lock().unwrap()[0].hash.clone();
    assert!(password::verify("new-password", &hash).is_ok());
    assert!(password::verify("old", &hash).is_err());
    assert!(matches!(auth::reset_password(&u, "ada@example.com", &token, "again").await, Err(ResetError::InvalidToken)), "used up");
}

#[tokio::test(flavor = "multi_thread")]
async fn tokens_expire_and_the_throttle_lapses() {
    let u = users();
    let (_, token) = auth::reset_link_token(&u, "ada@example.com").await.unwrap();
    let hour_ago = now() - auth::RESET_EXPIRES.as_secs() as i64;
    u.resets.lock().unwrap().get_mut(&1).unwrap().1 = hour_ago;
    assert!(matches!(auth::reset_password(&u, "ada@example.com", &token, "new-password").await, Err(ResetError::InvalidToken)), "expired");

    let (_, fresh) = auth::reset_link_token(&u, "ada@example.com").await.expect("old link no longer throttles");
    assert_ne!(fresh, token);
    auth::reset_password(&u, "ada@example.com", &fresh, "new-password").await.unwrap();
}
