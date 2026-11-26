# db

Typed `sqlx` pool handles and explicit transactions.

```toml
[dependencies]
db = { git = "https://github.com/ProjectLighthouse/cloudfoundry", features = ["postgres"] }
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

## Features

| Feature | Adds |
| --- | --- |
| `postgres`, `mysql`, `sqlite` | The `sqlx` driver |
| `singleton` | `Db::<H>::set_singleton` / `singleton()`: one process-wide slot per handle |
