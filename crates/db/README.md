# db

Typed `sqlx` pool handles and explicit transactions.

```toml
[dependencies]
db = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["postgres"] }
```

```rust
db::handle!(pub Primary: Postgres);

let db: db::Db<Primary> = db::Db::connect(&url).await?;
sqlx::query("select 1").execute(db.pool()).await?;
db.tx(async |tx| {
    sqlx::query("insert into runs default values").execute(&mut **tx).await?;
    Ok::<_, sqlx::Error>(())
}).await?;                                              // commits on Ok, rolls back on Err
```

Queries stay plain `sqlx`. A `Db<Analytics>` where `Db<Primary>` is expected does not compile.

## Config

`Db::connect_with(url, &cfg)` builds the pool from `PoolConfig` (defaults are sqlx's). An
impossible config (0 max, min above max, zero timeouts) is `sqlx::Error::Configuration`.

```toml
[db]
max_connections = 20
acquire_timeout = "5s"
max_lifetime = "never"
```

## Features

| Feature | Adds |
| --- | --- |
| `postgres`, `mysql`, `sqlite` | The `sqlx` driver |
| `configured` | `db::PoolConfig` deserializes (durations as `"30s"`, `"never"` for no limit) |
| `uuid` | `db::Uuid<T>`: a typed UUIDv7 for URLs and JSON (`uuid` / `blob` / `binary(16)` columns), so internal ids stay internal |
| `ts` | `ts_rs::TS` for `Uuid<T>`: `Uuid<"Team">` in generated TypeScript, a branded string declared in `Uuid.ts` (`art new` ships it in `resources/js/types/`) |
| `singleton` | `Db::<H>::set_singleton` / `singleton()`: one process-wide slot per handle; both return `Result<_, SingletonError>` |
