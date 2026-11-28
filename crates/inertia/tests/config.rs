use std::time::Duration;

use inertia::Config;

#[allow(clippy::unwrap_used)]
fn temp(name: &str, body: &str) -> String {
    let path = std::env::temp_dir().join(format!("inertia-config-{}-{name}", std::process::id()));
    std::fs::write(&path, body).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn root_template_errors_instead_of_falling_back() {
    let missing = Config { root_template: Some("/nonexistent/app.html".into()), ..Config::default() };
    assert!(inertia::layer().config(missing).is_err());

    let no_marker = Config { root_template: Some(temp("bad.html", "<html></html>")), ..Config::default() };
    assert!(inertia::layer().config(no_marker).is_err());

    let ok = Config { root_template: Some(temp("ok.html", "<body>{{ inertia }}</body>")), ..Config::default() };
    assert!(inertia::layer().config(ok).is_ok());
}

#[test]
fn empty_strings_mean_unset() {
    let cfg = Config { root_template: Some(String::new()), ssr_url: Some(String::new()), ..Config::default() };
    assert!(inertia::layer().config(cfg).is_ok());
}

#[test]
fn zero_ssr_timeout_is_rejected() {
    assert!(inertia::layer().config(Config { ssr_timeout: Duration::ZERO, ..Config::default() }).is_err());
}

#[cfg(not(feature = "ssr"))]
#[test]
fn ssr_url_needs_the_feature() {
    let cfg = Config { ssr_url: Some("http://127.0.0.1:13714".into()), ..Config::default() };
    assert!(inertia::layer().config(cfg).is_err());
}

#[cfg(feature = "ssr")]
#[test]
fn ssr_url_must_be_http() {
    let with = |url: &str| inertia::layer().config(Config { ssr_url: Some(url.into()), ..Config::default() });
    assert!(with("http://127.0.0.1:13714").is_ok());
    assert!(with("http://127.0.0.1:13714/").is_ok());
    for bad in ["https://ssr.example", "127.0.0.1:13714", "not a url"] {
        assert!(with(bad).is_err(), "{bad}");
    }
}

#[cfg(feature = "configured")]
mod serde_tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct App {
        #[serde(default)]
        inertia: Config,
    }

    #[test]
    fn partial_keeps_defaults() {
        let app: App = toml::from_str("[inertia]\nencrypt_history = true\nssr_timeout = \"500ms\"\n").unwrap();
        let want = Config { encrypt_history: true, ssr_timeout: Duration::from_millis(500), ..Config::default() };
        assert_eq!(app.inertia, want);
        assert_eq!(toml::from_str::<App>("").unwrap().inertia, Config::default());
    }

    #[test]
    fn unknown_key_is_rejected() {
        assert!(toml::from_str::<App>("[inertia]\nssr = \"x\"\n").is_err());
    }
}
