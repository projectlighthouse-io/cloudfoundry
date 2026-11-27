# auth

Know who the current user is, and guard routes.

```toml
[dependencies]
auth = { git = "https://github.com/projectlighthouse-io/cloudfoundry" }
```

```rust
impl auth::Provider for Users {
    type User = User;
    type Id = i64;
    async fn find(&self, id: i64) -> Result<Option<User>, auth::Error> { /* query */ }
    fn id(&self, user: &User) -> i64 { user.id }
}

router.layer(session_layer).layer(auth::layer(users));
r.group("/runs", |r| { r.layer(auth::require::<User>()); /* ... */ });

async fn me(user: auth::Authed<User>) -> String { user.email.clone() }
async fn login((auth, input): (auth::Auth<User>, Valid<Json<Login>>)) -> Result<Redirect, Errors> {
    auth.attempt(&users, &input.email, &input.password, input.remember).await?; // map AttemptError to your errors
    Ok(Redirect::to_route("home"))
}
```

`attempt` needs `auth::Credentials` (find by login, read and store the password hash). It
checks the password off the async runtime, upgrades hashes made with older settings, and
logs in. With feature `throttle`, `.throttle(limiter)` on the layer limits failed attempts per
login and IP (`Limiter::memory(FixedWindow::new(5, Duration::from_secs(60)))` is Laravel's default).

"Remember me": implement `auth::Remember` and add `.remember(codec)`; then
`auth.login_remember(&user)`. API tokens: implement `auth::Tokens` (find by
hash, store, delete) and add `.tokens()`; `Authorization: Bearer` requests are authenticated
without a session. `auth::issue_token(&users, &user, "cli", Some(ttl))` returns the plain token
once and stores only its SHA-256 hash; expired tokens are rejected; `auth::revoke_token(&users,
auth.bearer_token().unwrap())` logs an API client out.

Password reset: implement `auth::Resets` (one hashed token per user), then
`auth::reset_link_token(&users, email)` makes the link token (throttled to one a minute) and
`auth::reset_password(&users, email, token, new_password)` checks it (valid for an hour) and sets
the password. Sending the mail is the app's.

Password change: `auth.change_password(&users, current, new)` checks the current password, stores
the new hash, gives the session a new id and clears the "remember me" token.

`art publish auth [--driver sqlite|mysql] [--with-ui]` copies login, registration, password reset and password change handlers
(JSON by default; Inertia handlers and Vue pages with `--with-ui`), the
`users`/`api_tokens`/`password_resets` migration and a `Provider` into the app, to own and edit.
