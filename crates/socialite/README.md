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
