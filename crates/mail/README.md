# mail

Send mail from Markdown-ish templates over SMTP (lettre), to the log, or into memory for tests, now or from a
`queue` worker. `Mailer` is a trait (`SmtpMailer`, `LogMailer`, `MemoryMailer`); `mail::from_url` picks one:
`smtp://user:pass@host:587?tls=required`, `smtps://...`, `log`, `memory`.

```toml
[dependencies]
mail = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["configured", "queue"] }
```

```rust
let mailer = mail::from_url("smtp://127.0.0.1:1025", "Acme <hello@acme.test>")?;
let template = mail::Template::parse(include_str!("../resources/mail/verify-email.md"))?;
mailer.send(template.message("ada@acme.test", &[("link", &link)])?).await?;
mail::send_later::<App, _>(&queue, message).await?;      // feature `queue`; register `mail::SendMail<App>` on the worker

// Tests
let mailer = mail::MemoryMailer::new("hello@acme.test");
assert_eq!(mailer.sent()[0].subject, "Verify your email address");
```

Templates: `Subject:` line, blank line, then `#`/`##` headings, `- ` lists, paragraphs, `**bold**`, `` `code` ``,
`[text](url)`; a paragraph that is only a link is a button. `{{name}}` values are filled after rendering (escaped
in HTML); a missing one is an error. Every mail has a text and an HTML body.

`art publish mail` adds `src/mailer.rs`: the app's mailer from `[mail]` (`url` secret, `from`, `queue`) and
`deliver(app, template, to, vars)`, which queues by default. auth (reset, verification) and teams (invitations)
send through it. One SMTP connection per mail (no pool).

Tests: `cargo test -p mail --features configured,queue`; with `MAIL_SMTP_URL` and `MAIL_API_URL` pointing at a
Mailpit (`docker run -p 1025:1025 -p 8025:8025 axllent/mailpit`), one test delivers over SMTP and reads it back.

Docs: https://toolkit.projectlighthouse.io/docs/crates/mail.md
