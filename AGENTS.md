# toolkit

Small, independent Rust crates for server-driven web apps, plus the `art` CLI. Laravel is the
reference for developer experience. This is batteries, not a framework: no glue crate, no
re-export crate, no ORM, query builder, controllers or templating.

Docs (also as Markdown for agents): https://toolkit.projectlighthouse.io/llms.txt

## Rules

- Each crate stands alone and is usable as a git dependency: `route = { git = "https://github.com/projectlighthouse-io/toolkit" }`. No crates.io.
- Crates don't depend on each other unless they must; an integration is a Cargo feature on one side (e.g. `route/command` needs only `clap`). `art` depends on none of the crates.
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

Store conformance tests (`crates/{session,queue,throttle,teams}/tests/stores.rs`) run SQLite always, and
Postgres / MySQL when `*_PG_URL` / `*_MYSQL_URL` point at `postgres:16-alpine` / `mysql:8.4`; the session
Redis store when `SESSION_REDIS_URL` points at `redis:7-alpine`.

Smoke-test `art new` and `art publish` output by scaffolding outside the workspace and building it.

## Architecture notes

- `route`: `Router<S>` collects routes, groups and layers; `build(state)` composes each route's tower layers once (first added = outermost) and freezes into `Routes`. Handlers take up to six arguments: head-only inputs (`FromRequestParts`: `Cx<S>`, `Path`, `Session`, ...) first, at most one body-reading `FromRequest` input last. A tuple of inputs is also one input.
- Layers find types from other crates in request extensions: the session layer inserts `Session`; csrf reads a `TokenStore`; `auth::layer` inserts the provider; route inserts `Matched` and `UrlFor`. Order: session, csrf, auth, inertia.
- `validate`: `Valid<T>` only via `Valid::check`; rules are `Rule<T>` values, attached by `#[derive(Validate)]` (crate `validate-derive`, default feature `derive`); with feature `route`, `Errors` is a `422` response.
- `teams`: roles are app enums, each permission an app type with a `Grants` impl; `Can<P, R>` / `OrgCan<P, O>` read `{team}` / `{org}` (404 for non-members, 403 without `P`). `teams::layer` goes after `auth::layer`; rules live in `Teams`, stores only keep data. `CurrentTeam` is `users.current_team_id` (the teams migration alters the app's users table, no FK) cached in the session, falling back to the first team and storing it. `Teams::require_team(name)` creates a personal team on first `CurrentTeam` and makes leaving the last team `Error::LastTeam`. Invitation tokens are stored hashed: a link can be shown only right after `invite` / `resend_*`.
- `art make:*` writes one file and appends one `pub mod` line; `art new` templates go through `.replace("{name}", ..)`, including the frontend files in `art/templates/` (stylesheet, layouts, components; `include_str!`). Published `--with-ui` pages import them, and a test checks every relative import in the scaffold resolves. Keep `crates/*/stubs/pages` in step with the starter app's pages.
- Known rustc HRTB `Send` issue: a generic service projection naming `Request<route::Body>` fails `Send`; make layers generic over the request body.

## Commits

One-line, imperative messages; stage files by name; one logical change per commit, with tests.
