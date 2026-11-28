#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
//! Browsers & devices, `mark_confirmed`, email verification (and changing the
//! address), `Verified`, and the events with feature `event`.

use std::sync::{Arc, Mutex};

use auth::{Auth, Authed, Credentials, NewVerification, Provider, RecentlyConfirmed, Verification, Verifications, Verified, VerifyError};
use http_body_util::BodyExt;
use route::{
    Path, Router, Routes, empty,
    http::{Method, Request, StatusCode, header},
};
use securecookie::{Codec, Keys};
use session::{Devices, MemoryStore, Record, SessionId, Store};

const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

#[derive(Clone, Debug)]
struct User {
    id: i64,
    email: String,
    verified_at: Option<i64>,
}

#[derive(Default)]
struct Data {
    users: Vec<User>,
    /// (link, token hash, created_at)
    links: Vec<(Verification<i64>, Vec<u8>, i64)>,
}

#[derive(Clone, Default)]
struct Users(Arc<Mutex<Data>>);

impl Provider for Users {
    type User = User;
    type Id = i64;
    async fn find(&self, id: i64) -> Result<Option<User>, auth::Error> {
        Ok(self.0.lock().unwrap().users.iter().find(|u| u.id == id).cloned())
    }
    fn id(&self, user: &User) -> i64 {
        user.id
    }
}

impl Credentials for Users {
    async fn by_login(&self, login: &str) -> Result<Option<User>, auth::Error> {
        Ok(self.0.lock().unwrap().users.iter().find(|u| u.email == login).cloned())
    }
    fn password_hash(&self, _: &User) -> Option<String> {
        None
    }
    async fn set_password_hash(&self, _: i64, _: String) -> Result<(), auth::Error> {
        Ok(())
    }
}

impl Verifications for Users {
    fn email(&self, user: &User) -> String {
        user.email.clone()
    }
    fn email_verified_at(&self, user: &User) -> Option<i64> {
        user.verified_at
    }
    async fn store_verification(&self, l: NewVerification<i64>) -> Result<(), auth::Error> {
        let mut d = self.0.lock().unwrap();
        let id = d.links.len() as i64 + 1;
        d.links.push((Verification { id, user: l.user, email: l.email, expires_at: l.expires_at, verified_at: None }, l.token_hash, l.created_at));
        Ok(())
    }
    async fn last_verification_at(&self, id: i64) -> Result<Option<i64>, auth::Error> {
        Ok(self.0.lock().unwrap().links.iter().filter(|(l, ..)| l.user == id).map(|(.., at)| *at).max())
    }
    async fn verification_by_token(&self, hash: &[u8]) -> Result<Option<Verification<i64>>, auth::Error> {
        Ok(self.0.lock().unwrap().links.iter().find(|(_, h, _)| h == hash).map(|(l, ..)| l.clone()))
    }
    async fn complete_verification(&self, link: i64, user: i64, email: &str, now: i64) -> Result<bool, auth::Error> {
        let mut d = self.0.lock().unwrap();
        let Some((l, ..)) = d.links.iter_mut().find(|(l, ..)| l.id == link && l.verified_at.is_none()) else { return Ok(false) };
        l.verified_at = Some(now);
        let u = d.users.iter_mut().find(|u| u.id == user).unwrap();
        (u.email, u.verified_at) = (email.to_owned(), Some(now));
        Ok(true)
    }
}

impl Users {
    fn with(users: &[(i64, &str, bool)]) -> Self {
        let u = Users::default();
        u.0.lock().unwrap().users = users.iter().map(|&(id, email, v)| User { id, email: email.into(), verified_at: v.then_some(1) }).collect();
        u
    }
    /// Pretend the last link was made long ago, past the resend limit.
    fn age_links(&self) {
        for (.., at) in &mut self.0.lock().unwrap().links {
            *at -= 3600;
        }
    }
}

/// The session layer owns its store; the test reads the same one.
#[derive(Clone, Default)]
struct Sessions(Arc<MemoryStore>);

impl Store for Sessions {
    async fn load(&self, id: &SessionId) -> Result<Option<Record>, session::Error> {
        self.0.load(id).await
    }
    async fn save(&self, r: &Record) -> Result<(), session::Error> {
        self.0.save(r).await
    }
    async fn delete(&self, id: &SessionId) -> Result<(), session::Error> {
        self.0.delete(id).await
    }
}

impl Devices for Sessions {
    async fn devices(&self, user: i64) -> Result<Vec<session::Device>, session::Error> {
        self.0.devices(user).await
    }
    async fn revoke(&self, user: i64, device: uuid::Uuid) -> Result<bool, session::Error> {
        self.0.revoke(user, device).await
    }
    async fn revoke_others(&self, user: i64, keep: uuid::Uuid) -> Result<u64, session::Error> {
        self.0.revoke_others(user, keep).await
    }
}

type Log = Arc<Mutex<Vec<String>>>;

fn app(users: Users, sessions: Sessions, log: Log) -> Routes {
    let mut r = Router::new();
    let u = users.clone();
    r.post("/login/{id}", move |(auth, Path(id)): (Auth<User>, Path<i64>)| {
        let u = u.clone();
        async move {
            auth.login(&u.find(id).await.unwrap().unwrap());
            "in"
        }
    });
    r.post("/logout", |auth: Auth<User>| async move {
        auth.logout();
        "out"
    });
    r.get("/me", |user: Authed<User>| async move { user.email.clone() });
    let s = sessions.clone();
    r.get("/devices", move |(_, session): (Authed<User>, session::Session)| {
        let s = s.clone();
        async move {
            let current = session.public_id();
            let list = s.devices(session.user_id().unwrap()).await.unwrap();
            list.iter().map(|d| format!("{}{}", d.user_agent.as_deref().unwrap_or("?"), if Some(d.public_id) == current { "*" } else { "" })).collect::<Vec<_>>().join(",")
        }
    });
    let s = sessions.clone();
    r.delete("/devices/{ua}", move |(auth, Path(ua)): (Auth<User>, Path<String>)| {
        let s = s.clone();
        async move {
            let user = auth.user().unwrap().id;
            let found = s.devices(user).await.unwrap().into_iter().find(|d| d.user_agent.as_deref() == Some(ua.as_str()));
            auth.revoke_session(&s, found.map_or(uuid::Uuid::nil(), |d| d.public_id)).await.unwrap().to_string()
        }
    });
    let s = sessions.clone();
    r.delete("/devices", move |auth: Auth<User>| {
        let s = s.clone();
        async move { auth.revoke_other_sessions(&s).await.unwrap().to_string() }
    });
    r.post("/confirm-with-passkey", |auth: Auth<User>| async move {
        auth.mark_confirmed().unwrap();
        "confirmed"
    });
    r.get("/confirm-password", |_: ()| async { "confirm" }).name("password.confirm");
    r.get("/danger", |_: RecentlyConfirmed<User>| async { "danger" });
    r.get("/verify-email", |_: ()| async { "notice" }).name("verification.notice");
    r.get("/billing", |_: Verified<User>| async { "billing" });
    let u = users.clone();
    r.post("/verification/{email}", move |(auth, Path(email)): (Authed<User>, Path<String>)| {
        let u = u.clone();
        async move {
            let new = (email != "-").then_some(email.as_str());
            match auth::verification_token(&u, &auth, new).await {
                Ok(token) => token,
                Err(VerifyError::Throttled(_)) => "throttled".into(),
                Err(e) => e.to_string(),
            }
        }
    });
    let u = users.clone();
    r.get("/verify/{token}", move |(auth, Path(token)): (Auth<User>, Path<String>)| {
        let u = u.clone();
        async move {
            match auth.verify_email(&u, &token).await {
                Ok(user) => format!("verified {}", user.email),
                Err(e) => e.to_string(),
            }
        }
    });

    let layer = auth::layer(users).verification();
    #[cfg(feature = "event")]
    let layer = {
        use auth::events::{EmailVerified, SessionRevoked, SignedIn, SignedOut};
        let events = event::Events::<Log>::builder()
            .on(|log: Log, e: SignedIn| async move {
            log.lock().unwrap().push(format!("in {}", e.user_id));
            Ok(())
        })
            .on(|log: Log, e: SignedOut| async move {
            log.lock().unwrap().push(format!("out {}", e.user_id));
            Ok(())
        })
            .on(|log: Log, e: SessionRevoked| async move {
            log.lock().unwrap().push(format!("revoked {} {}", e.user_id, e.count));
            Ok(())
        })
            .on(|log: Log, e: EmailVerified| async move {
            log.lock().unwrap().push(format!("verified {} {} {}", e.user_id, e.email, e.changed));
            Ok(())
        })
            .build();
        layer.events(Arc::new(events), log)
    };
    #[cfg(not(feature = "event"))]
    drop(log);
    r.layer(session::layer(sessions, Codec::new(Keys::parse(KEY).unwrap()))).layer(layer);
    r.build(()).unwrap()
}

/// A browser: its own user agent and session cookie.
struct Browser<'a> {
    r: &'a Routes,
    ua: &'static str,
    cookie: Option<String>,
}

impl Browser<'_> {
    /// Body for 200, `"<status> <location>"` otherwise.
    async fn send(&mut self, method: Method, uri: &str, json: bool) -> String {
        let mut req = Request::builder().method(method).uri(uri).header(header::USER_AGENT, self.ua);
        if json {
            req = req.header(header::ACCEPT, "application/json");
        }
        if let Some(c) = &self.cookie {
            req = req.header(header::COOKIE, c.as_str());
        }
        let res = self.r.handle(req.body(empty()).unwrap()).await;
        if let Some(set) = res.headers().get(header::SET_COOKIE) {
            self.cookie = Some(set.to_str().unwrap().split(';').next().unwrap().to_owned());
        }
        let status = res.status();
        let location = res.headers().get(header::LOCATION).map(|v| v.to_str().unwrap().to_owned()).unwrap_or_default();
        let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        if status == StatusCode::OK { body } else { format!("{} {location}", status.as_u16()).trim().to_owned() }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn browsers_and_devices() {
    let (sessions, log) = (Sessions::default(), Log::default());
    let r = app(Users::with(&[(1, "ada@acme.test", true), (2, "bob@acme.test", true)]), sessions.clone(), log.clone());
    let (mut firefox, mut safari, mut phone) = (Browser { r: &r, ua: "Firefox", cookie: None }, Browser { r: &r, ua: "Safari", cookie: None }, Browser { r: &r, ua: "Phone", cookie: None });
    for b in [&mut firefox, &mut safari, &mut phone] {
        b.send(Method::POST, "/login/1", true).await;
    }
    let mut bob = Browser { r: &r, ua: "Bob", cookie: None };
    bob.send(Method::POST, "/login/2", true).await;

    let list = firefox.send(Method::GET, "/devices", true).await;
    assert_eq!(list.split(',').count(), 3, "{list}");
    assert!(list.contains("Firefox*") && list.contains("Safari") && !list.contains("Bob"), "{list}: this one marked, only ada's");

    assert_eq!(firefox.send(Method::DELETE, "/devices/Bob", true).await, "false", "not hers");
    assert_eq!(firefox.send(Method::DELETE, "/devices/Safari", true).await, "true");
    assert_eq!(safari.send(Method::GET, "/me", true).await, "401", "revoked: logged out");
    assert_eq!(phone.send(Method::GET, "/me", true).await, "ada@acme.test");

    assert_eq!(firefox.send(Method::DELETE, "/devices", true).await, "1", "all but this one");
    assert_eq!(phone.send(Method::GET, "/me", true).await, "401");
    assert_eq!(firefox.send(Method::GET, "/devices", true).await, "Firefox*");
    assert_eq!(bob.send(Method::GET, "/me", true).await, "bob@acme.test", "other users untouched");

    firefox.send(Method::POST, "/logout", true).await;
    assert!(sessions.devices(1).await.unwrap().is_empty(), "logout ends the device");
    if cfg!(feature = "event") {
        let log = log.lock().unwrap().clone();
        assert_eq!(log, ["in 1", "in 1", "in 1", "in 2", "revoked 1 1", "revoked 1 1", "out 1"]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn confirmed_by_passkey_without_a_password() {
    let r = app(Users::with(&[(1, "ada@acme.test", true)]), Sessions::default(), Log::default());
    let mut b = Browser { r: &r, ua: "Firefox", cookie: None };
    b.send(Method::POST, "/login/1", true).await;
    assert_eq!(b.send(Method::GET, "/danger", true).await, "423");
    assert_eq!(b.send(Method::GET, "/danger", false).await, "303 /confirm-password");
    b.send(Method::POST, "/confirm-with-passkey", true).await;
    assert_eq!(b.send(Method::GET, "/danger", true).await, "danger");
}

#[tokio::test(flavor = "multi_thread")]
async fn email_verification_and_change() {
    let (users, log) = (Users::with(&[(1, "ada@acme.test", false), (2, "bob@acme.test", true)]), Log::default());
    let r = app(users.clone(), Sessions::default(), log.clone());
    let mut b = Browser { r: &r, ua: "Firefox", cookie: None };
    assert_eq!(b.send(Method::GET, "/billing", true).await, "401");
    b.send(Method::POST, "/login/1", true).await;
    assert_eq!(b.send(Method::GET, "/billing", true).await, "403", "JSON: forbidden until verified");
    assert_eq!(b.send(Method::GET, "/billing", false).await, "303 /verify-email", "page visit: to the notice");

    let token = b.send(Method::POST, "/verification/-", true).await;
    assert_eq!(b.send(Method::POST, "/verification/-", true).await, "throttled", "one link a minute");
    assert!(b.send(Method::GET, "/verify/wrong", true).await.contains("invalid"));
    // The link works in any browser: the token is the proof.
    let mut other = Browser { r: &r, ua: "Phone", cookie: None };
    assert_eq!(other.send(Method::GET, &format!("/verify/{token}"), true).await, "verified ada@acme.test");
    assert!(other.send(Method::GET, &format!("/verify/{token}"), true).await.contains("invalid"), "used once");
    assert_eq!(b.send(Method::GET, "/billing", true).await, "billing");
    users.age_links();
    assert_eq!(b.send(Method::POST, "/verification/-", true).await, VerifyError::AlreadyVerified.to_string());

    // Changing the address: verify the new one first.
    assert_eq!(b.send(Method::POST, "/verification/bob@acme.test", true).await, VerifyError::EmailTaken.to_string());
    let token = b.send(Method::POST, "/verification/ada@new.test", true).await;
    assert_eq!(b.send(Method::GET, "/me", true).await, "ada@acme.test", "unchanged until verified");
    assert_eq!(b.send(Method::GET, &format!("/verify/{token}"), true).await, "verified ada@new.test");
    assert_eq!(b.send(Method::GET, "/me", true).await, "ada@new.test");
    if cfg!(feature = "event") {
        let log = log.lock().unwrap().clone();
        assert_eq!(log, ["in 1", "verified 1 ada@acme.test false", "verified 1 ada@new.test true"]);
    }
}
