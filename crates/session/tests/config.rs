#![cfg(feature = "configured")]

use std::time::Duration;

use session::{Config, SameSite};

#[derive(serde::Deserialize)]
struct App {
    #[serde(default)]
    session: Config,
}

#[test]
fn partial_keeps_defaults() {
    let app: App = toml::from_str("[session]\nlifetime = \"2h\"\nsame_site = \"strict\"\n").unwrap();
    let want = Config { lifetime: Duration::from_secs(7200), same_site: SameSite::Strict, ..Config::default() };
    assert_eq!(app.session, want);
}

#[test]
fn missing_section_is_default() {
    let app: App = toml::from_str("").unwrap();
    assert_eq!(app.session, Config::default());
}

#[test]
fn unknown_key_is_rejected() {
    let err = toml::from_str::<App>("[session]\ncookie_nam = \"x\"\n").err().expect("typo must fail");
    assert!(err.to_string().contains("cookie_nam"), "{err}");
}

#[test]
fn round_trips() {
    let cfg = Config { cookie_name: "app".into(), lifetime: Duration::from_secs(90), ..Config::default() };
    let back: Config = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
    assert_eq!(back, cfg);
}

#[test]
fn layer_rejects_invalid_config() {
    let codec = securecookie::Codec::new(securecookie::Keys::parse(&securecookie::Keys::generate()).unwrap());
    let bad = Config { cookie_name: "a;b".into(), ..Config::default() };
    assert!(session::layer(session::MemoryStore::new(), codec).config(bad).is_err());
}

/// Through config-rs's deserializer, not `toml`: enums, `with` durations and
/// `deny_unknown_fields` must behave the same.
#[test]
fn loads_through_configured() {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct App {
        #[serde(default)]
        session: Config,
    }
    let dir = std::env::temp_dir().join(format!("session-configured-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("default.toml"), "[session]\nlifetime = \"2h\"\nsame_site = \"none\"\n").unwrap();
    let app = configured::load::<App>(&dir).unwrap().value;
    assert_eq!(app.session, Config { lifetime: Duration::from_secs(7200), same_site: SameSite::None, ..Config::default() });

    std::fs::write(dir.join("default.toml"), "[session]\nsame_site = \"sideways\"\n").unwrap();
    assert!(configured::load::<App>(&dir).is_err());
    std::fs::write(dir.join("default.toml"), "[session]\nlifetim = \"2h\"\n").unwrap();
    assert!(configured::load::<App>(&dir).is_err());
}
