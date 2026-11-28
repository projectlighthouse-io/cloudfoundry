#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
// `.into()` for the user handle: its type depends on webauthn-rs features unified in the workspace.
#![allow(clippy::useless_conversion)]
//! Real ceremonies against webauthn-rs' software authenticator (`SoftPasskey`).

use passkeys::{Error, MemoryStore, Passkeys, RequestChallengeResponse};
use session::Session;
use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};
use webauthn_rs::prelude::Url;
use webauthn_rs_proto::AllowCredentials;

const ORIGIN: &str = "https://acme.test";

fn origin() -> Url {
    Url::parse(ORIGIN).unwrap()
}

fn passkeys() -> Passkeys<MemoryStore> {
    Passkeys::new(MemoryStore::new(), ORIGIN, "Acme").unwrap()
}

/// Register a new software passkey for `user`.
async fn register(pk: &Passkeys<MemoryStore>, key: &mut SoftPasskey, user: i64, name: &str) -> passkeys::Passkey {
    let session = Session::new();
    let options = pk.start_registration(&session, user, "ada@acme.test", "Ada").await.unwrap();
    let selection = options.public_key.authenticator_selection.as_ref().unwrap();
    assert_eq!(selection.resident_key, Some(webauthn_rs_proto::ResidentKeyRequirement::Preferred), "discoverable, for passwordless");
    let answer = key.do_registration(origin(), options).unwrap();
    pk.finish_registration(&session, user, name, &answer).await.unwrap()
}

/// SoftPasskey only signs for credentials in `allowCredentials`; a browser
/// would find a discoverable one on its own.
fn allow(mut options: RequestChallengeResponse, credential_id: &[u8]) -> RequestChallengeResponse {
    options.public_key.allow_credentials.push(AllowCredentials { type_: "public-key".into(), id: credential_id.to_vec().into(), transports: None });
    options
}

#[tokio::test]
async fn registration_is_single_use_and_per_user() {
    let pk = passkeys();
    let mut key = SoftPasskey::new(true);
    let added = register(&pk, &mut key, 1, "  MacBook  ").await;
    assert_eq!((added.user_id, added.name.as_str(), added.last_used_at), (1, "MacBook", None));

    let session = Session::new();
    let options = pk.start_registration(&session, 1, "ada@acme.test", "Ada").await.unwrap();
    let excluded = options.public_key.exclude_credentials.clone().unwrap();
    assert_eq!(excluded.len(), 1, "existing passkeys are excluded");
    let answer = key.do_registration(origin(), options).unwrap();
    assert!(matches!(pk.finish_registration(&session, 2, "x", &answer).await, Err(Error::NoCeremony)), "started for user 1");
    assert!(matches!(pk.finish_registration(&session, 1, "x", &answer).await, Err(Error::NoCeremony)), "state is gone after one try");

    // A second passkey keeps the user's handle; another user gets their own.
    let second = register(&pk, &mut SoftPasskey::new(true), 1, "Phone").await;
    let other = register(&pk, &mut SoftPasskey::new(true), 2, "Laptop").await;
    assert_eq!(second.user_handle, added.user_handle);
    assert_ne!(other.user_handle, added.user_handle);
    assert_eq!(pk.passkeys(1).await.unwrap().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Phone", "MacBook"], "newest first");
}

#[tokio::test]
async fn second_factor() {
    let pk = passkeys();
    let mut key = SoftPasskey::new(true);
    let added = register(&pk, &mut key, 1, "Key").await;

    assert!(matches!(pk.start_authentication(&Session::new(), 2).await, Err(Error::NoPasskeys)));

    let session = Session::new();
    let options = pk.start_authentication(&session, 1).await.unwrap();
    let answer = key.do_authentication(origin(), options).unwrap();
    assert!(matches!(pk.finish_authentication(&session, None, 2, &answer).await, Err(Error::NoCeremony)), "another user's ceremony");

    let options = pk.start_authentication(&session, 1).await.unwrap();
    let answer = key.do_authentication(origin(), options).unwrap();
    let used = pk.finish_authentication(&session, None, 1, &answer).await.unwrap();
    assert_eq!(used.public_id, added.public_id);
    assert!(used.last_used_at.is_some());
    assert!(pk.passkeys(1).await.unwrap()[0].last_used_at.is_some(), "stored");
    assert!(matches!(pk.finish_authentication(&session, None, 1, &answer).await, Err(Error::NoCeremony)), "no replay");

    // An answer to another session's challenge does not verify.
    let (s1, s2) = (Session::new(), Session::new());
    let first = pk.start_authentication(&s1, 1).await.unwrap();
    pk.start_authentication(&s2, 1).await.unwrap();
    let answer = key.do_authentication(origin(), first).unwrap();
    assert!(matches!(pk.finish_authentication(&s2, None, 1, &answer).await, Err(Error::Rejected(_))));
}

#[tokio::test]
async fn passwordless_login() {
    let pk = passkeys();
    let mut key = SoftPasskey::new(true);
    let added = register(&pk, &mut key, 7, "Key").await;

    let session = Session::new();
    let options = pk.start_login(&session).unwrap();
    assert!(options.public_key.allow_credentials.is_empty(), "no username: the browser picks");
    let mut answer = key.do_authentication(origin(), allow(options, &added.credential_id)).unwrap();
    // A discoverable credential returns the user handle; SoftPasskey does not store one.
    answer.response.user_handle = Some(added.user_handle.as_bytes().to_vec().into());
    let used = pk.finish_login(&session, None, &answer).await.unwrap();
    assert_eq!((used.user_id, used.public_id), (7, added.public_id));
    assert!(matches!(pk.finish_login(&session, None, &answer).await, Err(Error::NoCeremony)), "no replay");

    // The handle must match the stored credential's.
    let session = Session::new();
    let options = pk.start_login(&session).unwrap();
    let mut answer = key.do_authentication(origin(), allow(options, &added.credential_id)).unwrap();
    answer.response.user_handle = Some(uuid::Uuid::new_v4().as_bytes().to_vec().into());
    assert!(matches!(pk.finish_login(&session, None, &answer).await, Err(Error::UnknownCredential)));

    // A removed passkey no longer signs in.
    assert!(pk.remove(7, added.public_id).await.unwrap());
    let session = Session::new();
    let options = pk.start_login(&session).unwrap();
    let mut answer = key.do_authentication(origin(), allow(options, &added.credential_id)).unwrap();
    answer.response.user_handle = Some(added.user_handle.as_bytes().to_vec().into());
    assert!(matches!(pk.finish_login(&session, None, &answer).await, Err(Error::UnknownCredential)));
}

#[tokio::test]
async fn rename_and_remove_are_per_user() {
    let pk = passkeys();
    let added = register(&pk, &mut SoftPasskey::new(true), 1, "Key").await;
    assert!(!pk.rename(2, added.public_id, "Mine").await.unwrap());
    assert!(pk.rename(1, added.public_id, " Work key ").await.unwrap());
    assert_eq!(pk.passkeys(1).await.unwrap()[0].name, "Work key");
    assert!(!pk.remove(2, added.public_id).await.unwrap());
    assert!(pk.remove(1, added.public_id).await.unwrap());
    assert!(pk.passkeys(1).await.unwrap().is_empty());
}

#[cfg(feature = "event")]
#[tokio::test]
async fn emits_events() {
    use std::sync::{Arc, Mutex};

    use passkeys::events::{PasskeyAdded, PasskeyRemoved, PasskeyUsed};

    type Log = Arc<Mutex<Vec<String>>>;
    let log: Log = Arc::default();
    let events = event::Events::<Log>::builder()
        .on(|log: Log, e: PasskeyAdded| async move {
            log.lock().unwrap().push(format!("added {} {}", e.user_id, e.name));
            Ok(())
        })
        .on(|log: Log, e: PasskeyUsed| async move {
            log.lock().unwrap().push(format!("used {} {}", e.user_id, e.passwordless));
            Ok(())
        })
        .on(|log: Log, e: PasskeyRemoved| async move {
            log.lock().unwrap().push(format!("removed {}", e.user_id));
            Ok(())
        })
        .build();
    let pk = passkeys().events(Arc::new(events), log.clone());

    let mut key = SoftPasskey::new(true);
    let added = register(&pk, &mut key, 1, "Key").await;
    let session = Session::new();
    let answer = key.do_authentication(origin(), pk.start_authentication(&session, 1).await.unwrap()).unwrap();
    pk.finish_authentication(&session, None, 1, &answer).await.unwrap();
    assert!(!pk.remove(2, added.public_id).await.unwrap(), "not theirs: no event");
    pk.remove(1, added.public_id).await.unwrap();
    assert_eq!(*log.lock().unwrap(), ["added 1 Key", "used 1 false", "removed 1"]);
}

#[cfg(feature = "throttle")]
#[tokio::test]
async fn throttles_failed_answers() {
    use std::{net::IpAddr, sync::Arc, time::Duration};

    let limiter = Arc::new(throttle::Limiter::memory(throttle::FixedWindow::new(2, Duration::from_secs(60))));
    let pk = passkeys().throttle(limiter);
    let mut key = SoftPasskey::new(true);
    let added = register(&pk, &mut key, 1, "Key").await;
    let (ip, other): (IpAddr, IpAddr) = ("10.0.0.1".parse().unwrap(), "10.0.0.2".parse().unwrap());

    // A sign-in answer for this credential, with the right or a wrong user handle.
    let answer = |session: &Session, key: &mut SoftPasskey, right: bool| {
        let options = pk.start_login(session).unwrap();
        let mut answer = key.do_authentication(origin(), allow(options, &added.credential_id)).unwrap();
        let handle = if right { added.user_handle } else { uuid::Uuid::new_v4() };
        answer.response.user_handle = Some(handle.as_bytes().to_vec().into());
        answer
    };
    for _ in 0..2 {
        let session = Session::new();
        let bad = answer(&session, &mut key, false);
        assert!(matches!(pk.finish_login(&session, Some(ip), &bad).await, Err(Error::UnknownCredential)));
    }
    let session = Session::new();
    let good = answer(&session, &mut key, true);
    assert!(matches!(pk.finish_login(&session, Some(ip), &good).await, Err(Error::Throttled(_))), "over the limit: even a good answer waits");
    let session = Session::new();
    let good = answer(&session, &mut key, true);
    assert_eq!(pk.finish_login(&session, Some(other), &good).await.unwrap().user_id, 1, "per IP");

    // Second factors count per user and IP; a success clears the count.
    let session = Session::new();
    let bad = key.do_authentication(origin(), pk.start_authentication(&session, 1).await.unwrap()).unwrap();
    pk.start_authentication(&session, 1).await.unwrap(); // a new challenge: the old answer no longer matches
    assert!(matches!(pk.finish_authentication(&session, Some(ip), 1, &bad).await, Err(Error::Rejected(_))));
    let good = key.do_authentication(origin(), pk.start_authentication(&session, 1).await.unwrap()).unwrap();
    pk.finish_authentication(&session, Some(ip), 1, &good).await.unwrap();
    for _ in 0..2 {
        let bad = key.do_authentication(origin(), pk.start_authentication(&session, 1).await.unwrap()).unwrap();
        pk.start_authentication(&session, 1).await.unwrap();
        assert!(matches!(pk.finish_authentication(&session, Some(ip), 1, &bad).await, Err(Error::Rejected(_))), "cleared by the success");
    }
    let good = key.do_authentication(origin(), pk.start_authentication(&session, 1).await.unwrap()).unwrap();
    assert!(matches!(pk.finish_authentication(&session, Some(ip), 1, &good).await, Err(Error::Throttled(_))));
}
