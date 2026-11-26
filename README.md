# foundry

Small, independent Rust crates for server-driven web apps (Inertia + Vue), and
`art`, a CLI to scaffold and run them. Laravel is the reference for developer
experience, Gorilla for structure: no framework crate, no service container, no
ORM. Each crate stands alone; the wiring is a `main.rs` the app owns.

```bash
cargo install --git https://github.com/projectlighthouse-io/cloudfoundry art
art new myapp --git https://github.com/projectlighthouse-io/cloudfoundry
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
| [`password`](crates/password) | Argon2id hashing |
| [`auth`](crates/auth) | Current user, `Authed<U>`, guards, publishable login |
| [`socialite`](crates/socialite) | OAuth login (GitHub) |
| [`inertia`](crates/inertia) | The Inertia.js v3 protocol |
| [`vite`](crates/vite) | Asset tags from Vite |
| [`queue`](crates/queue) | Jobs: push, claim, retry, failed jobs |
| [`event`](crates/event) | Typed in-process events, inline or queued listeners |
| [`sse`](crates/sse) | Server-sent event responses |
| [`art`](art) | The CLI |

Crates ship as git dependencies: `route = { git = "https://github.com/projectlighthouse-io/cloudfoundry" }`.
Rust 1.85+ (`art`: 1.88+). Development: `cargo test --workspace --all-features`.
