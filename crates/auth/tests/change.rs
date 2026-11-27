use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use auth::{Auth, Authed, ChangeError, Credentials, Provider, Remember};
use http_body_util::BodyExt;
use route::{
    Path, Router, Routes, empty,
    http::{Method, Request, StatusCode, header},
};
use securecookie::{Codec, Keys};

#[derive(Clone)]
struct User {
    id: u32,
    hash: String,
}

#[derive(Clone)]
struct Users {
    users: Arc<Mutex<Vec<User>>>,
    remember: Arc<Mutex<HashMap<u32, String>>>,
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
        Ok((login == "ada").then(|| self.users.lock().unwrap()[0].clone()))
    }
    fn password_hash(&self, user: &User) -> String {
        user.hash.clone()
    }
    async fn set_password_hash(&self, id: u32, hash: String) -> Result<(), auth::Error> {
        self.users.lock().unwrap().iter_mut().find(|u| u.id == id).unwrap().hash = hash;
        Ok(())
    }
}

impl Remember for Users {
    fn remember_token(&self, user: &User) -> Option<String> {
        self.remember.lock().unwrap().get(&user.id).cloned()
    }
    async fn set_remember_token(&self, id: u32, token: Option<String>) -> Result<(), auth::Error> {
        let mut m = self.remember.lock().unwrap();
        match token {
            Some(t) => m.insert(id, t),
            None => m.remove(&id),
        };
        Ok(())
    }
}

#[derive(serde::Deserialize)]
struct Change {
    current: String,
    new: String,
}

fn app(users: Users) -> Routes {
    let codec = || Codec::new(Keys::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap());
    let mut r = Router::new();
    let u = users.clone();
    r.post("/login", move |auth: Auth<User>| {
        let u = u.clone();
        async move { auth.attempt(&u, "ada", "old password", true).await.map(|_| "in").unwrap_or("failed") }
    });
    let u = users.clone();
    r.post("/password/{current}/{new}", move |(auth, Path(c)): (Auth<User>, Path<Change>)| {
        let u = u.clone();
        async move {
            match auth.change_password(&u, &c.current, &c.new).await {
                Ok(()) => "changed",
                Err(ChangeError::WrongPassword) => "wrong",
                Err(ChangeError::NotLoggedIn) => "anonymous",
                Err(ChangeError::Other(_)) => "error",
            }
        }
    });
    r.get("/me", |_: Authed<User>| async { "ada" });
    r.layer(session::layer(session::MemoryStore::new(), codec())).layer(auth::layer(users).remember(codec()));
    r.build(()).unwrap()
}

/// Send with cookies; return the body (or status) and the Set-Cookie pairs.
async fn send(r: &Routes, method: Method, uri: &str, cookies: &[&str]) -> (String, Vec<String>) {
    let mut req = Request::builder().method(method).uri(uri).header(header::ACCEPT, "application/json");
    if !cookies.is_empty() {
        req = req.header(header::COOKIE, cookies.join("; "));
    }
    let res = r.handle(req.body(empty()).unwrap()).await;
    let set = res.headers().get_all(header::SET_COOKIE).iter().map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned()).collect();
    let status = res.status();
    let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    (if status == StatusCode::OK { body } else { status.as_str().to_owned() }, set)
}

fn named(set: &[String], name: &str) -> Option<String> {
    set.iter().find(|c| c.starts_with(&format!("{name}="))).cloned()
}

#[tokio::test(flavor = "multi_thread")]
async fn change_password_checks_the_current_one_and_rotates_credentials() {
    let users = Users {
        users: Arc::new(Mutex::new(vec![User { id: 1, hash: password::hash("old password").unwrap() }])),
        remember: Arc::default(),
    };
    let r = app(users.clone());

    assert_eq!(send(&r, Method::POST, "/password/x/y", &[]).await.0, "anonymous");

    let (body, set) = send(&r, Method::POST, "/login", &[]).await;
    assert_eq!(body, "in");
    let session = named(&set, "session").unwrap();
    let remember = named(&set, "remember").unwrap();
    assert!(users.remember.lock().unwrap().contains_key(&1));

    assert_eq!(send(&r, Method::POST, "/password/nope/new-password", &[&session]).await.0, "wrong");
    assert!(password::verify("old password", &users.users.lock().unwrap()[0].hash).is_ok(), "unchanged");

    let (body, set) = send(&r, Method::POST, "/password/old%20password/new-password", &[&session, &remember]).await;
    assert_eq!(body, "changed");
    assert!(password::verify("new-password", &users.users.lock().unwrap()[0].hash).is_ok());
    let fresh = named(&set, "session").expect("new session cookie");
    assert_ne!(fresh, session);
    assert_eq!(named(&set, "remember").as_deref(), Some("remember="), "remember cookie cleared");
    assert!(users.remember.lock().unwrap().is_empty(), "remember token cleared");

    assert_eq!(send(&r, Method::GET, "/me", &[&fresh]).await.0, "ada", "still logged in here");
    assert_eq!(send(&r, Method::GET, "/me", &[&session]).await.0, "401", "old session id is dead");
    assert_eq!(send(&r, Method::GET, "/me", &[&remember]).await.0, "401", "old remember cookie is dead");
}
