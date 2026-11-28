# factor2

Two-factor authentication, Laravel Fortify-style: TOTP codes from an authenticator app, single-use backup
codes, and "trust this browser for 30 days". Works with `auth` and `session`; the login and settings handlers
are published into the app.

```toml
[dependencies]
factor2 = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["sqlite", "throttle"] }
crypt = { git = "https://github.com/projectlighthouse-io/toolkit" }
```

```sh
art publish auth --with-ui --driver sqlite      # users, login, sudo mode (confirm password)
art publish factor2 --with-ui --driver sqlite   # tables, handlers, settings + challenge pages
art migrate
```

```rust
let tf = TwoFactor::new(factor2::SqliteStore::new(pool), crypt, cookie_codec, "Acme")
    .throttle(Arc::new(throttle::Limiter::memory(throttle::FixedWindow::new(5, Duration::from_secs(60)))));

// Settings
let setup = tf.setup(user.id, &user.email).await?;      // secret, otpauth:// URI, QR code (SVG); unconfirmed
let backup_codes = tf.confirm(user.id, &code).await?;    // on; show the 10 codes once
tf.regenerate_backup_codes(user.id).await?;              // behind auth::PasswordConfirmed
tf.disable(user.id).await?;                              // behind auth::PasswordConfirmed

// Login (the published `handlers::two_factor::login` replaces `handlers::login::login`)
let user = auth.validate(&users, &email, &password).await?;          // password only, no login
if tf.enabled(user.id).await? && !tf.trusted(user.id, &device).await? {
    tf.begin(&session, user.id, remember)?;                           // pending for 5 minutes
    return Ok(Redirect::to_route("two-factor.login"));
}
auth.login(&user);

// Challenge
let passed = tf.challenge(&session, ip, Code::Totp(&code)).await?;   // or Code::Backup(..)
auth.login(&users.find(passed.user_id).await?.unwrap());             // rotates the session id
```

## Rules

- TOTP is SHA-1, 6 digits, 30 seconds; a code is accepted for the current step and one either side. Each step
  works once: the last used step is stored and checked in the same `update`, so a replayed code (or two racing
  requests with one code) fails.
- Secrets are sealed with `crypt` (purpose `factor2/totp:<user id>`): a database dump alone does not yield them,
  and a sealed secret does not open for another user's row.
- Backup codes (`xxxxxxxxxx-xxxxxxxxxx`, 100 bits) are stored as SHA-256 and work once; regenerating replaces
  them all. Input ignores case, spaces and dashes.
- A trusted browser holds a `securecookie`-signed cookie `two_factor_trusted` naming the user and a random token;
  only the token's hash is stored, with the user agent and IP for the settings list. Revoking a device, or
  disabling two-factor, stops the cookie working.
- With `.throttle(limiter)`, wrong codes count per user and IP; past the limit even a right code waits.
- Disabling two-factor and regenerating codes are for `auth::PasswordConfirmed<User>` handlers (sudo mode).
- Events for the app to emit (the stubs do): `TwoFactorEnabled`, `TwoFactorDisabled`,
  `TwoFactorChallengeFailed`, `BackupCodeUsed`.
- No cascades: call `disable(user_id)` before deleting a user.

Tables: `totp_secrets`, `backup_codes`, `trusted_devices` (times are unix seconds; `trusted_devices.uuid` names
a device in URLs and JSON). Store conformance tests run SQLite always, Postgres / MySQL with `FACTOR2_PG_URL` /
`FACTOR2_MYSQL_URL`.

## Features

| Feature | Adds |
| --- | --- |
| `sqlite`, `postgres`, `mysql` | `SqliteStore`, `PostgresStore`, `MysqlStore` |
| `throttle` | `TwoFactor::throttle(limiter)` |
| `configured` | `factor2::Config` deserializes (`trust_for = "30d"`, `pending_for = "5m"`, `backup_codes`, `secure`) |
| `ts` | `ts_rs::TS` for `TrustedDevice`, `Setup`, `Status` (page props) |
