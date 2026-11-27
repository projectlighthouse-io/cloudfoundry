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

Store conformance tests (`crates/{session,queue,throttle}/tests/stores.rs`) run SQLite always, and
Postgres / MySQL when `*_PG_URL` / `*_MYSQL_URL` point at `postgres:16-alpine` / `mysql:8.4`.

Smoke-test `art new` and `art publish` output by scaffolding outside the workspace and building it.

## Architecture notes

- `route`: `Router<S>` collects routes, groups and layers; `build(state)` composes each route's tower layers once (first added = outermost) and freezes into `Routes`. Handlers are `fn(I)` or `fn(Cx<S>, I)` with one `FromRequest` input; head-only inputs implement `FromRequestParts`, which lets them sit in tuple inputs.
- Layers find types from other crates in request extensions: the session layer inserts `Session`; csrf reads a `TokenStore`; `auth::layer` inserts the provider; route inserts `Matched` and `UrlFor`. Order: session, csrf, auth, inertia.
- `validate`: `Valid<T>` only via `Valid::check`; with feature `route`, `Errors` is a `422` response.
- `art make:*` writes one file and appends one `pub mod` line; `art new` templates go through `.replace("{name}", ..)`.
- Known rustc HRTB `Send` issue: a generic service projection naming `Request<route::Body>` fails `Send`; make layers generic over the request body.

## Commits

One-line, imperative messages; stage files by name; one logical change per commit, with tests.
