# passkeys

Passkeys (WebAuthn, on [webauthn-rs](https://github.com/kanidm/webauthn-rs) 0.6): passwordless sign-in with
discoverable credentials, and a passkey as the second factor on the `factor2` challenge. Works with `auth`
and `session`; the handlers, settings page and browser helper are published into the app.

```toml
[dependencies]
passkeys = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["sqlite", "event", "throttle"] }
```

```sh
art publish auth --with-ui --driver sqlite       # users, login, sudo mode (confirm password)
art publish factor2 --with-ui --driver sqlite    # optional: the challenge a passkey can answer
art publish passkeys --with-ui --driver sqlite   # table, handlers, settings page, resources/js/passkeys.ts
art migrate
```

Every ceremony is two requests: `start_*` returns options for `navigator.credentials.create/get` and keeps
the state in the session (5 minutes, used once); `finish_*` checks the browser's `credential.toJSON()`.

```rust
let pk = Passkeys::new(passkeys::SqliteStore::new(pool), &cfg.app_url, &cfg.app_name)?  // rp id = APP_URL's host
    .throttle(limiter)                                                                // feature `throttle`
    .events(app.events.clone(), app.clone());                                         // feature `event`

// Settings (signed in)
let options = pk.start_registration(&session, user.id, &user.email, &user.email).await?;
let added = pk.finish_registration(&session, user.id, "MacBook", &credential).await?;
pk.rename(user.id, added.public_id, "Work laptop").await?;
pk.remove(user.id, added.public_id).await?;             // behind auth::RecentlyConfirmed

// Passwordless
let options = pk.start_login(&session)?;
let used = pk.finish_login(&session, Some(ip), &credential).await?;   // then auth.login(user used.user_id)

// Second factor, on the two-factor challenge
let user = tf.pending(&session).unwrap();
let options = pk.start_authentication(&session, user).await?;
pk.finish_authentication(&session, Some(ip), user, &credential).await?;
let passed = tf.pass(&session)?;                              // then auth.login; `trust` sets factor2's cookie
```

- Registration asks for a discoverable credential (`residentKey: preferred`) and excludes the user's
  existing passkeys. Each user has one random WebAuthn user handle, shared by their passkeys.
- User verification (PIN, biometrics) is required, so a passwordless sign-in skips the two-factor challenge.
- Counters and backup flags are saved after each use; a cloned authenticator's counter is rejected by webauthn-rs.
- The stored credential is webauthn-rs' `Passkey`, as JSON (`jsonb` / `text` / `json`).
- Ceremony state is serialised into the session (webauthn-rs feature `danger-allow-state-serialisation`).
  Keep the session server side (memory, SQL or Redis store), never in a client-side cookie.

Feature `throttle`: `Passkeys::throttle(limiter)` counts failed passwordless sign-ins per IP and failed
second factors per user and IP (a success clears the count); over the limit, `Error::Throttled(retry_after)`.

Feature `event`: `Passkeys::events(events, cx)` emits `PasskeyAdded`, `PasskeyRemoved` and `PasskeyUsed`
on the `event` bus. A failing listener is logged, not returned. Without the feature the structs in
`passkeys::events` are there for the app to emit.

Docs: https://toolkit.projectlighthouse.io/docs/modules/passkeys.md

Stores: `MemoryStore`; `SqliteStore`, `PostgresStore`, `MysqlStore` behind features. Conformance tests
run Postgres/MySQL with `PASSKEYS_PG_URL` / `PASSKEYS_MYSQL_URL`. Before deleting a user, call
`delete_all(user_id)`: there are no cascades.
