# foundry

Small, independent Rust crates for server-driven web apps (Inertia + Vue), and
`art`, a CLI to scaffold and run them. Laravel is the reference for developer
experience, Gorilla for structure: no framework crate, no service container, no
ORM. Each crate stands alone; the wiring is a `main.rs` the app owns.

```bash
cargo install --git https://github.com/projectlighthouse-io/toolkit art
art new myapp --git https://github.com/projectlighthouse-io/toolkit
```

| Crate | Does |
| --- | --- |
| [`route`](crates/route) | Match a request to a handler; named routes, groups, tower layers |
| [`db`](crates/db) | Typed `sqlx` pool handles, explicit transactions |
| [`command`](crates/command) | Named commands in the app binary |
| [`action`](crates/action) | One trait for a unit of application logic |
| [`validate`](crates/validate) | `Validate`, `Valid<T>`, field errors |
| [`securecookie`](crates/securecookie) | Sign and encrypt cookie values |
| [`session`](crates/session) | Sessions: layer, flash, Postgres / MySQL / SQLite stores |
| [`csrf`](crates/csrf) | CSRF tokens (Inertia's `X-XSRF-TOKEN`) |
| [`password`](crates/password) | Argon2id hashing, rehash checks |
| [`throttle`](crates/throttle) | Rate limits per key: fixed/sliding window, token bucket; middleware, logins, outgoing calls |
| [`auth`](crates/auth) | Current user, guards, `attempt` with throttling, API tokens with expiry, password reset; publishable handlers |
| [`socialite`](crates/socialite) | OAuth login: GitHub, GitLab, Google, Reddit, X, Auth0, any OpenID Connect provider (one feature each) |
| [`teams`](crates/teams) | Organizations and teams with typed roles and permissions, invitations |
| [`inertia`](crates/inertia) | The Inertia.js v3 protocol |
| [`vite`](crates/vite) | Asset tags from Vite |
| [`queue`](crates/queue) | Jobs: push, claim, retry, failed jobs |
| [`event`](crates/event) | Typed in-process events, inline or queued listeners |
| [`sse`](crates/sse) | Server-sent event responses |
| [`art`](art) | The CLI |

## Versioning

Everything is beta: each crate has its own `0.x.y` version in its `Cargo.toml`, bumped when that crate
changes (in `0.x`, a minor bump may break the API). There are no releases or tags yet; apps depend
on the repository and `Cargo.lock` pins the commit. The docs site shows each crate's version.

Crates ship as git dependencies: `route = { git = "https://github.com/projectlighthouse-io/toolkit" }`.
Rust 1.85+ (`art`: 1.88+). Development: `cargo test --workspace --all-features`.
