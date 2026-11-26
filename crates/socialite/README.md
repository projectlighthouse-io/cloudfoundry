# socialite

OAuth login with third-party providers (GitHub).

```toml
[dependencies]
socialite = { git = "https://github.com/ProjectLighthouse/cloudfoundry" }
```

```rust
let github = socialite::GitHub::new(client_id, client_secret, "https://app/auth/github/callback")
    .scopes(&["read:user"]);

// GET /auth/github
Redirect::to(github.authorize_url(&session))
// GET /auth/github/callback
let gh_user = github.user(&session, callback).await?;   // checks `state`, trades the code
```
