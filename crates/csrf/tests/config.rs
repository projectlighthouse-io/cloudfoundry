#![cfg(feature = "configured")]

use csrf::Config;

#[derive(serde::Deserialize)]
struct App {
    #[serde(default)]
    csrf: Config,
}

#[test]
fn parses_and_defaults() {
    assert_eq!(toml::from_str::<App>("").unwrap().csrf, Config::default());
    assert!(!toml::from_str::<App>("[csrf]\nsecure = false\n").unwrap().csrf.secure);
}

#[test]
fn unknown_key_is_rejected() {
    assert!(toml::from_str::<App>("[csrf]\nsecur = false\n").is_err());
}
