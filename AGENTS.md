# toolkit

Small, independent Rust crates for server-driven web apps, plus the `art` CLI. Laravel is the
reference for developer experience. This is batteries, not a framework: no glue crate, no
re-export crate, no ORM, query builder, controllers or templating.

Docs (also as Markdown for agents): https://toolkit.projectlighthouse.io/llms.txt

## Rules

- Each crate stands alone and is usable as a git dependency: `route = { git = "https://github.com/projectlighthouse-io/toolkit" }`. No crates.io.
- Crates don't depend on each other unless they must; an integration is a Cargo feature on one side (e.g. `route/command` needs only `clap`). `art` depends on none of the crates. The one shared dependency is `configured` (no default features), behind each crate's `configured` feature.
- No crate reads env vars or files. Settings with defaults go in a `pub struct Config` (derives through `configured::serde`, `default` + `deny_unknown_fields`, durations via `configured::duration`), applied with `.config(cfg)`, which returns `Result` when a value can be invalid and never panics. Required values (stores, codecs, keys, URLs) stay constructor params; builders take `&str`, `Config` fields are `String`.
- Modules (`auth`, `socialite`, ...) ship app-owned code as stubs under `stubs/`, declared in `[package.metadata.art.publish]`; `art publish` copies them. The crate never calls them.
- Every database feature supports SQLite, Postgres and MySQL; build and test them in that order.
- Library crates log through `tracing`, never `eprintln!` (CLI output excepted).
- Everything is beta: each crate has its own `0.x.y` version in its `Cargo.toml`.
- No example or CI app in this repo.

## Commands

```bash
cargo check --workspace --all-features --tests
cargo check -p validate                  # one crate alone, default features (--workspace hides missing features)
cargo test -p route                      # one crate
cargo test -p route --test route urls    # one test
cargo test -p db --features sqlite       # db tests need a driver feature
cargo run -p art -- new demo             # scaffold an app (path deps on this checkout)
```

Store conformance tests (`crates/{session,queue,throttle,teams,factor2}/tests/stores.rs`) run SQLite always, and
Postgres / MySQL when `*_PG_URL` / `*_MYSQL_URL` point at `postgres:16-alpine` / `mysql:8.4`; the session
Redis store when `SESSION_REDIS_URL` points at `redis:7-alpine`.

Smoke-test `art new` and `art publish` output by scaffolding outside the workspace and building it.

## Architecture notes

- `route`: `Router<S>` collects routes, groups and layers; `build(state)` composes each route's tower layers once (first added = outermost) and freezes into `Routes`. Handlers take up to six arguments: head-only inputs (`FromRequestParts`: `Cx<S>`, `Path`, `Session`, ...) first, at most one body-reading `FromRequest` input last. A tuple of inputs is also one input.
- Layers find types from other crates in request extensions: the session layer inserts `Session`; csrf reads a `TokenStore`; `auth::layer` inserts the provider; route inserts `Matched` and `UrlFor`. Order: session, csrf, auth, inertia.
- `validate`: `Valid<T>` only via `Valid::check`; rules are `Rule<T>` values, attached by `#[derive(Validate)]` (crate `validate-derive`, default feature `derive`); with feature `route`, `Errors` is a `422` response.
- `teams`: roles are app enums, each permission an app type with a `Grants` impl; `Can<P, R>` / `OrgCan<P, O>` read `{team}` / `{org}` as a `uuid` (`db::Uuid`; 404 for numeric ids, unknown uuids and non-members, 403 without `P`). Internal `i64` ids are `#[serde(skip)]`: never in URLs or JSON; `route::Bind` + `Bound<T>` give app models the same path binding. Published page props derive `ts_rs::TS` and Vue pages import the generated types (`db/ts` brands uuids as `Uuid<"Team">`); the generated `.ts` files ship as stubs (`crates/*/stubs/types/`): after changing a stub prop struct, regenerate them by scaffolding, publishing and running `cargo test export_bindings`, then copy them back. `teams::layer` goes after `auth::layer`; rules live in `Teams`, stores only keep data. `CurrentTeam` is `users.current_team_id` (the teams migration alters the app's users table, no FK) cached in the session, falling back to the first team and storing it. `Teams::require_team(name)` creates a personal team on first `CurrentTeam` and makes leaving the last team `Error::LastTeam`. Invitation tokens are stored hashed: a link can be shown only right after `invite` / `resend_*`.
- `auth` sudo mode: `Auth::confirm_password` stamps the session; `PasswordConfirmed<U>` passes within `Config::password_timeout`, else redirects page visits to `Config::confirm_route` (remembering the `GET` path or same-origin `Referer` for `Auth::intended`) and answers JSON with `423`. `Auth::validate` is `attempt` without the login, for a second factor.
- `crypt`: `Encrypter` seals bytes at rest (XChaCha20-Poly1305, key derived from APP_KEY with its own label, so it can share APP_KEY with `securecookie`; `purpose` is the AAD; key id in the header for rotation, `needs_rotation`). No timestamps, unlike `securecookie`.
- `factor2`: `TwoFactor<S: Store>` holds the rules (enrol/confirm/disable, challenge, backup codes, trusted browsers) and never logs in: the published `handlers::two_factor::login` (replacing auth's on `POST /login`) runs `auth.validate`, then `auth.login` or `TwoFactor::begin` (pending login in the session); `challenge` finishes with `auth.login`. Replay protection is the store's `use_step` (one conditional `update`). Secrets sealed with `crypt` per user id; backup codes and device tokens stored as SHA-256. The app builds one `TwoFactor` in a `OnceLock` (`src/two_factor.rs`) so the memory limiter lives for the app. Events are plain structs in `factor2::events`, emitted by the stubs.
- `art make:*` writes one file and appends one `pub mod` line; `art new` templates go through `.replace("{name}", ..)`, including the frontend files in `art/templates/` (stylesheet, layouts, components; `include_str!`). Published `--with-ui` pages import them, and a test checks every relative import in the scaffold resolves. Keep `crates/*/stubs/pages` in step with the starter app's pages.
- `configured::load::<T>("config")` (app only) reads `.env`, `config/default.*`, `config/<APP_ENV>.*`, resolves `env(X[, default])`, `required_env(X)`, `secret_env(X)`, `required_secret_env(X)`, reports every problem at once and redacts secrets in `Loaded.listing.show()` (`<redacted>`; `show_secret_values()` only behind `config:show --print-secret-values`; the command captures the startup listing, no reload) and in errors. Secret fields are `configured::SecretString`.
- Known rustc HRTB `Send` issue: a generic service projection naming `Request<route::Body>` fails `Send`; make layers generic over the request body.

## Commits

One-line, imperative messages; stage files by name; one logical change per commit, with tests.
