//! Custom validators are plain functions, closures or inline conditions.

use validate::{Errors, Message, OrMessage, Valid, Validate, alpha_dash, max_len, nullable, required, rule, same};

/// A reusable rule: any function returning `Result<(), Message>`.
fn repo_slug(s: &str) -> Result<(), Message> {
    let parts: Vec<&str> = s.split('/').collect();
    rule(parts.len() == 2 && parts.iter().all(|p| alpha_dash(p).is_ok()), "Must look like owner/name.")
}

struct Signup {
    name: String,
    password: String,
    password_confirmation: String,
    repo: String,
    bio: Option<String>,
    tags: Vec<String>,
}

impl Validate for Signup {
    fn validate(&self, e: &mut Errors) {
        e.check("name", required(&self.name).or_message("Tell us your name."));
        e.check("repo", repo_slug(&self.repo));
        // Cross-field: the whole struct is in scope.
        e.check("password_confirmation", same(&self.password, &self.password_confirmation));
        e.check("bio", nullable(self.bio.as_deref(), |b| max_len(b, 10)));
        // Per-item errors, keyed the way a form names its inputs.
        for (i, tag) in self.tags.iter().enumerate() {
            e.check(&format!("tags.{i}"), required(tag));
        }
        // Inline condition with its own message.
        e.check("tags", rule(self.tags.len() <= 3, "Pick at most 3 tags."));
    }
}

fn signup() -> Signup {
    Signup {
        name: "ada".into(),
        password: "pw".into(),
        password_confirmation: "pw".into(),
        repo: "ada/engine".into(),
        bio: None,
        tags: vec!["rust".into()],
    }
}

#[test]
fn valid_input_passes() {
    assert!(Valid::check(signup()).is_ok());
}

#[test]
fn custom_rules_report_their_messages() {
    let bad = Signup {
        name: " ".into(),
        password_confirmation: "nope".into(),
        repo: "not a slug".into(),
        bio: Some("far too long a bio".into()),
        tags: vec!["".into(), "b".into(), "c".into(), "d".into()],
        ..signup()
    };
    let e = Valid::check(bad).err().unwrap();
    assert_eq!(e.get("name"), ["Tell us your name."]);
    assert_eq!(e.get("repo"), ["Must look like owner/name."]);
    assert_eq!(e.get("password_confirmation"), ["Must match."]);
    assert_eq!(e.get("bio"), ["Must be at most 10 characters."]);
    assert_eq!(e.get("tags.0"), ["This field is required."]);
    assert_eq!(e.get("tags"), ["Pick at most 3 tags."]);
}
