# session

Load and save session data for a request.

```toml
[dependencies]
session = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["postgres", "route", "csrf"] }
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
session [--driver sqlite|mysql]`. `MemoryStore` and SQLite are single-server; run several
instances on Postgres, MySQL or Redis.

## Config

`.config(cfg)` sets every option and returns `Err(ConfigError)` for an invalid
cookie name, a lifetime under a second, or `SameSite=None` without `Secure`.
With feature `configured`, load it with the `configured` crate (missing keys keep defaults,
unknown keys fail):

```toml
[session]
cookie_name = "app_session"
lifetime = "2h"
same_site = "strict"
secure = true
```

## Features

| Feature | Adds |
| --- | --- |
| `postgres`, `mysql`, `sqlite` | `PostgresStore`, `MysqlStore`, `SqliteStore` (`MemoryStore` always) |
| `redis` | `RedisStore` (Redis or Valkey; keys expire on their own, no table). Every app instance pointed at the same server shares sessions |
| `route` | `Session` as a handler input |
| `csrf` | `Session` implements `csrf::TokenStore` |
| `command` | `session::command::prune` for `session:prune` |
| `configured` | `Config` and `SameSite` deserialize; durations as `"14d"`, `"2h"` |
