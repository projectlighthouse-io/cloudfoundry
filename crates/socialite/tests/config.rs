#![cfg(all(feature = "configured", feature = "github", feature = "gitlab", feature = "auth0"))]
#![allow(clippy::unwrap_used)]

use socialite::{Auth0, GitHub, GitLab, Provider, ProviderConfig};

fn cfg(extra: &str) -> ProviderConfig {
    let base = "client_id = \"id\"\nclient_secret = \"s3cr3t\"\nredirect_url = \"https://app.test/auth/github/callback\"\n";
    toml::from_str(&format!("{base}{extra}")).unwrap()
}

#[test]
fn builds_providers_with_scopes_and_base() {
    let gh = GitHub::from_config(cfg("scopes = [\"read:user\"]\n")).unwrap();
    assert_eq!(gh.oauth().scopes, ["read:user"]);
    assert_eq!(gh.oauth().client_secret, "s3cr3t");
    assert_eq!(gh.oauth().authorize_url, "https://github.com/login/oauth/authorize");

    let gl = GitLab::from_config(cfg("base_url = \"https://git.acme.test/\"\n")).unwrap();
    assert_eq!(gl.oauth().authorize_url, "https://git.acme.test/oauth/authorize");

    let a0 = Auth0::from_config(cfg("base_url = \"https://acme.eu.auth0.com\"\nscopes = [\"openid\"]\n")).unwrap();
    assert_eq!(a0.oauth().token_url, "https://acme.eu.auth0.com/oauth/token");
    assert_eq!(a0.oauth().scopes, ["openid"]);
}

#[test]
fn rejects_configs_that_cannot_work() {
    assert!(GitHub::from_config(cfg("base_url = \"https://ghe.test\"\n")).is_err(), "github takes no base_url");
    assert!(Auth0::from_config(cfg("")).is_err(), "auth0 needs base_url");
    assert!(GitHub::from_config(cfg("scopes = [\"\"]\n")).is_err());
    let bad = |toml: &str| GitHub::from_config(toml::from_str(toml).unwrap()).is_err();
    assert!(bad("client_id = \"\"\nclient_secret = \"s\"\nredirect_url = \"https://a.test/cb\"\n"));
    assert!(bad("client_id = \"i\"\nclient_secret = \" \"\nredirect_url = \"https://a.test/cb\"\n"));
    assert!(bad("client_id = \"i\"\nclient_secret = \"s\"\nredirect_url = \"/auth/github/callback\"\n"));
    assert!(bad("client_id = \"i\"\nclient_secret = \"s\"\nredirect_url = \"https:///cb\"\n"));
}

#[test]
fn unknown_keys_fail_and_debug_hides_the_secret() {
    assert!(toml::from_str::<ProviderConfig>("client_id = \"i\"\nclient_secret = \"s\"\nredirect_url = \"https://a.test\"\nscope = []\n").is_err());
    assert!(!format!("{:?}", cfg("")).contains("s3cr3t"));
}
