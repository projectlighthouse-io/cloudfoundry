# session

Load and save session data for a request.

```toml
[dependencies]
session = { git = "https://github.com/projectlighthouse-io/cloudfoundry", features = ["postgres", "route", "csrf"] }
```

```rust
let sessions = session::layer(session::PostgresStore::new(pool), codec)
    .cookie_name("app_session")
    .lifetime(Duration::from_secs(14 * 86400));
router.layer(sessions);

async fn count(s: session::Session) -> String {
    let n = s.get::<u32>("n")?.unwrap_or(0) + 1;
    s.insert("n", &n)?;
    n.to_string()
}
```

Lazy: an untouched session writes nothing and sets no cookie. Flash data lives one
request. `regenerate()` on login, `invalidate()` on logout. Tables: `art publish
session [--driver sqlite|mysql]`.

## Features

| Feature | Adds |
| --- | --- |
| `postgres`, `mysql`, `sqlite` | `PostgresStore`, `MysqlStore`, `SqliteStore` (`MemoryStore` always) |
| `route` | `Session` as a handler input |
| `csrf` | `Session` implements `csrf::TokenStore` |
| `command` | `session::command::prune` for `session:prune` |
