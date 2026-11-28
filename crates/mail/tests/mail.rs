#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
//! Drivers by URL, the memory driver, and real SMTP when MAIL_SMTP_URL and
//! MAIL_API_URL point at a Mailpit (`docker run -p 1025:1025 -p 8025:8025 axllent/mailpit`).

use mail::{Error, Mailer, Message};

#[tokio::test]
async fn memory_keeps_what_was_sent() {
    let mailer = Mailer::memory("Acme <hello@acme.test>");
    let clone = mailer.clone();
    clone.send(Message::new("ada@acme.test", "Hi", "text").html("<p>html</p>")).await.unwrap();
    assert_eq!(mailer.sent(), [Message { to: "ada@acme.test".into(), subject: "Hi".into(), text: "text".into(), html: Some("<p>html</p>".into()) }], "clones share");
    assert!(matches!(mailer.send(Message::new("not an address", "Hi", "x")).await, Err(Error::Address(_))));
    assert_eq!(mailer.sent().len(), 1);
}

#[test]
fn urls_pick_the_driver() {
    assert!(Mailer::from_url("log", "hello@acme.test").is_ok());
    assert!(Mailer::from_url("smtp://user:pw@127.0.0.1:1025", "Acme <hello@acme.test>").is_ok());
    assert!(Mailer::from_url("smtps://smtp.example.com", "hello@acme.test").is_ok());
    assert!(matches!(Mailer::from_url("http://x", "hello@acme.test"), Err(Error::Url(_))));
    assert!(matches!(Mailer::from_url("log", "nope"), Err(Error::Address(_))));
    let Err(e) = Mailer::from_url("smtp://user:s3cret@[bad", "hello@acme.test") else { panic!("bad url accepted") };
    assert!(!e.to_string().contains("s3cret"), "the password never shows: {e}");
    assert!(Mailer::from_url("memory", "x@y.z").unwrap().sent().is_empty());
}

#[cfg(feature = "configured")]
#[test]
fn config_reads_the_mail_section() {
    let cfg: mail::Config = toml::from_str("url = \"memory\"\nfrom = \"Acme <hello@acme.test>\"").unwrap();
    Mailer::config(&cfg).unwrap();
    let default: mail::Config = toml::from_str("").unwrap();
    assert_eq!(default.from, "hello@example.test");
    assert!(toml::from_str::<mail::Config>("nope = 1").is_err(), "unknown keys fail");
}

#[tokio::test]
async fn smtp_delivers() {
    let (Ok(smtp), Ok(api)) = (std::env::var("MAIL_SMTP_URL"), std::env::var("MAIL_API_URL")) else {
        return eprintln!("MAIL_SMTP_URL / MAIL_API_URL not set; skipping");
    };
    let subject = format!("toolkit mail test {}", std::process::id());
    let mailer = Mailer::from_url(&smtp, "Acme <hello@acme.test>").unwrap();
    mailer.send(Message::new("Ada <ada@acme.test>", &subject, "plain body").html("<b>html body</b>")).await.unwrap();
    // Mailpit's API: the newest messages, with subject and recipients.
    let out = std::process::Command::new("curl").args(["-s", &format!("{api}/api/v1/messages")]).output().unwrap();
    let list: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let found = list["messages"].as_array().unwrap().iter().find(|m| m["Subject"] == subject.as_str()).expect("delivered to Mailpit");
    assert_eq!(found["To"][0]["Address"], "ada@acme.test");
    assert_eq!(found["From"]["Address"], "hello@acme.test");
}
