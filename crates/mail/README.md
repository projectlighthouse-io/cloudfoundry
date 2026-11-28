# mail

Send mail over SMTP (lettre), to the log, or into memory for tests. The driver comes from a URL:
`smtp://user:pass@host:587?tls=required`, `smtps://...`, `log`, `memory`.

```toml
[dependencies]
mail = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["configured"] }
```

```rust
let mailer = mail::Mailer::from_url("smtp://127.0.0.1:1025", "Acme <hello@acme.test>")?;
mailer.send(mail::Message::new("ada@acme.test", "Verify your email address", "Open https://...").html("<a href=...>")).await?;

// Tests
let mailer = mail::Mailer::memory("hello@acme.test");
assert_eq!(mailer.sent()[0].subject, "Verify your email address");
```

`art publish mail` adds `src/mailer.rs` (the app's one `Mailer`, from `[mail]` in config: `url` is a secret,
`from`). A bad URL is logged and mail falls back to the log driver. Addresses are checked before sending on
every driver. One SMTP connection per mail (no pool); send bulk mail from a queued job.

Tests: `cargo test -p mail --features configured`; with `MAIL_SMTP_URL` and `MAIL_API_URL` pointing at a
Mailpit (`docker run -p 1025:1025 -p 8025:8025 axllent/mailpit`), one test delivers over SMTP and reads it back.

Docs: https://toolkit.projectlighthouse.io/docs/crates/mail.md
