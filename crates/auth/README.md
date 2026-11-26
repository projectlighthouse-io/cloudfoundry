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
async fn login((auth, input): (auth::Auth<User>, Valid<Json<Login>>)) -> Redirect { /* auth.login(&user) */ }
```

"Remember me": implement `auth::Remember` and add `.remember(codec)`; then
`auth.login_remember(&user)`. API tokens: implement `auth::Tokens` and add
`.tokens()`; `Authorization: Bearer` requests are authenticated without a session.

`art publish auth [--driver sqlite|mysql]` copies login handlers, the login page, a `users` migration and a
`Provider` into the app, to own and edit.
