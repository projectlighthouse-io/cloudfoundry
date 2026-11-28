use std::time::Duration;

use auth::Config;

#[test]
fn check_rejects_broken_settings() {
    assert!(Config::default().check().is_ok());
    assert!(Config { login_route: String::new(), ..Config::default() }.check().is_err());
    for name in ["", "a b", "a;b", "a=b"] {
        assert!(Config { remember_cookie: name.into(), ..Config::default() }.check().is_err(), "{name:?}");
    }
    assert!(Config { remember_for: Duration::ZERO, ..Config::default() }.check().is_err());
    assert!(Config { remember_for: Duration::from_secs(401 * 86400), ..Config::default() }.check().is_err());
    assert!(Config { remember_for: Duration::from_secs(30 * 86400), ..Config::default() }.check().is_ok());
}

#[cfg(feature = "configured")]
mod serde_tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct App {
        #[serde(default)]
        auth: Config,
    }

    #[test]
    fn partial_keeps_defaults() {
        let app: App = toml::from_str("[auth]\nremember_for = \"30d\"\nlogin_route = \"signin\"\n").unwrap();
        let want = Config { remember_for: Duration::from_secs(30 * 86400), login_route: "signin".into(), ..Config::default() };
        assert_eq!(app.auth, want);
        assert_eq!(toml::from_str::<App>("").unwrap().auth, Config::default());
    }

    #[test]
    fn unknown_key_is_rejected() {
        assert!(toml::from_str::<App>("[auth]\nremember_fro = \"1d\"\n").is_err());
    }
}
