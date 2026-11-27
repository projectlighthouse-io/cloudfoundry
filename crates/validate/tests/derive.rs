#![cfg(feature = "derive")]

//! `#[derive(Validate)]`: every attribute, renamed keys, `Option` fields, your
//! own rules, `with` functions and the messages table.

use serde::Deserialize;
use validate::{Errors, Message, Messages, Rule, Valid, Validate, min, rule};

struct RepoSlug;

impl Rule<str> for RepoSlug {
    fn name(&self) -> &str {
        "repo_slug"
    }
    fn check(&self, value: &str) -> Result<(), Message> {
        rule(value.split_once('/').is_some_and(|(o, n)| !o.is_empty() && !n.is_empty()), "Use owner/name.")
    }
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
#[validate(with = cross_checks, messages = messages)]
struct Signup {
    #[validate(required, max = 20)]
    full_name: String,
    #[validate(required, email)]
    email: String,
    #[validate(required, min = 8)]
    password: String,
    #[validate(same = password)]
    password_confirmation: String,
    #[validate(required, rule = RepoSlug)]
    repo: String,
    #[validate(nullable, url)]
    website: Option<String>,
    #[validate(required, between(13, 120))]
    age: Option<u8>,
    #[validate(one_of("free", "pro"))]
    plan: String,
    #[validate(max = 3)]
    tags: Vec<String>,
    #[serde(rename = "acceptTerms")]
    #[validate(accepted)]
    terms: bool,
    admin: bool,
    #[validate(with = team_rules)]
    team: String,
    permissions: Vec<String>,
}

fn team_rules(s: &Signup, team: &str) -> Result<(), Message> {
    if s.admin { rule(team == "staff", "Admins belong to the staff team.") } else { Ok(()) }
}

fn cross_checks(s: &Signup, e: &mut Errors) {
    if s.admin {
        e.field("permissions", &s.permissions).rule(min(1));
    }
}

fn messages() -> Messages {
    Messages::new().set("fullName.required", "Tell us your name.").set("repo.repo_slug", "Which repository? Use owner/name.")
}

fn good() -> Signup {
    Signup {
        full_name: "Ada Lovelace".into(),
        email: "ada@example.com".into(),
        password: "correct horse".into(),
        password_confirmation: "correct horse".into(),
        repo: "ada/engine".into(),
        website: None,
        age: Some(36),
        plan: "pro".into(),
        tags: vec!["rust".into()],
        terms: true,
        admin: false,
        team: "ops".into(),
        permissions: vec![],
    }
}

#[test]
fn valid_input_passes() {
    assert!(Valid::check(good()).is_ok());
}

#[test]
fn every_rule_reports_under_the_client_key() {
    let bad = Signup {
        full_name: " ".into(),
        email: "nope".into(),
        password: "short".into(),
        password_confirmation: "different".into(),
        repo: "acme".into(),
        website: Some("not a url".into()),
        age: Some(9),
        plan: "gold".into(),
        tags: vec!["a".into(); 4],
        terms: false,
        admin: true,
        team: "ops".into(),
        permissions: vec![],
    };
    let e = Valid::check(bad).unwrap_err();
    let got: Vec<(&str, &str)> = e.iter().map(|(k, m)| (k, m[0].as_ref())).collect();
    assert_eq!(
        got,
        [
            ("acceptTerms", "Must be accepted."),
            ("age", "Must be between 13 and 120."),
            ("email", "Must be a valid email address."),
            ("fullName", "Tell us your name."),
            ("password", "Must be at least 8 characters."),
            ("passwordConfirmation", "Does not match."),
            ("permissions", "Must have at least 1 item."),
            ("plan", "Must be one of: free, pro."),
            ("repo", "Which repository? Use owner/name."),
            ("tags", "Must have at most 3 items."),
            ("team", "Admins belong to the staff team."),
            ("website", "Must be a valid URL."),
        ]
    );
    assert!(e.iter().all(|(_, m)| m.len() == 1), "each field reports one message");
}

#[test]
fn option_fields_are_absent_unless_required() {
    let e = Valid::check(Signup { age: None, website: None, ..good() }).unwrap_err();
    assert_eq!(e.get("age"), ["This field is required."], "required Option<u8> that is None");
    assert!(e.get("website").is_empty(), "nullable Option<String> that is None");
}

#[test]
fn custom_rules_keep_their_own_message_without_an_entry() {
    #[derive(Debug, Validate)]
    struct Only {
        #[validate(rule = RepoSlug)]
        repo: String,
    }
    assert_eq!(Valid::check(Only { repo: "x".into() }).unwrap_err().get("repo"), ["Use owner/name."]);
}

#[test]
fn deserialized_json_validates_with_the_same_keys() {
    let json = r#"{"fullName":"","email":"ada@example.com","password":"correct horse","passwordConfirmation":"correct horse",
        "repo":"ada/engine","website":null,"age":30,"plan":"free","tags":[],"acceptTerms":true,"admin":false,"team":"x","permissions":[]}"#;
    let input: Signup = serde_json::from_str(json).unwrap();
    assert_eq!(Valid::check(input).unwrap_err().get("fullName"), ["Tell us your name."]);
}
