# socialite

OAuth login with third-party providers. `state` and PKCE are handled; turning the
provider's user into an app user is app code.

```toml
[dependencies]
socialite = { git = "https://github.com/projectlighthouse-io/cloudfoundry", features = ["google", "github"] }
```

| Feature | Provider |
| --- | --- |
| `github` | `GitHub` |
| `gitlab` | `GitLab` (gitlab.com or `.base_url(...)` for self-hosted) |
| `google` | `Google` (`.hosted_domain`, `.offline`, `.prompt`) |
| `reddit` | `Reddit` (no email) |
| `twitter` | `Twitter`, i.e. X (no email) |
| `oidc` | `Oidc::discover(name, issuer, ...)` or `Oidc::new(name, ..., endpoints)` |
| `auth0` | `Auth0::new(domain, ...)` (implies `oidc`) |
| `route` | `socialite::routes(&mut r, provider, callback)` and the `SocialCallback` input |

```rust
use socialite::{Google, Provider};

let google = Google::new(client_id, client_secret, "https://app/auth/google/callback");

// GET /auth/google
Redirect::to(google.authorize_url(&session))                 // stores state + PKCE verifier
// GET /auth/google/callback
let social = google.social_user(&session, callback).await?;  // checks state, trades the code
// social.provider == "google", social.id, social.email, social.email_verified, ...
```

`user(...)` returns the provider's own type (`GoogleUser`, `GitHubUser`, `OidcUser` with all
claims, ...); `social_user(...)` returns one `SocialUser` shape for every provider. Add a provider
by implementing `Provider` (endpoints in `OAuth2`, plus `fetch_user`).

## Routes and account linking

With feature `route`, `socialite::routes(&mut r, google, handlers::social::callback)` adds
`GET /auth/google` (redirects to Google) and `GET /auth/google/callback` (your handler, which takes a
`SocialCallback` holding the user or the error). Route names: `social.google`, `social.google.callback`.

`art publish socialite [--driver sqlite|mysql] [--with-ui]` (after `art publish auth`) copies:

- `src/social.rs`: `Users::find_or_create_social`. An account already linked to the provider account
  logs in; a logged-in user gets it linked; otherwise the same email is the same account; otherwise a
  new account without a password is created. Providers without email (Reddit, X) can only be
  connected by a logged-in user.
- `src/handlers/social.rs`: the callback (JSON apps redirect to `APP_URL`; `--with-ui` shows errors on
  the login page) and, with `--with-ui`, `SocialButtons.vue`.
- Migrations: `social_accounts`, and `users.password_hash` made nullable (on SQLite by rebuilding the
  table; list your columns there if you changed `users`).
