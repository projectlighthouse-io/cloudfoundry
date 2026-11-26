# command

Register and dispatch commands in the app binary.

```toml
[dependencies]
command = { git = "https://github.com/projectlighthouse-io/cloudfoundry" }
```

```rust
#[derive(clap::Args)]
struct PruneArgs { #[arg(long, default_value = "30")] days: u32 }

async fn prune(app: &App, args: PruneArgs) -> Result<(), AppError> { Ok(()) }

command::Commands::new()
    .default("serve", serve)
    .add("runs:prune", prune)
    .run(app, std::env::args())
    .await;                       // ./myapp runs:prune --days 7
```

Errors print to stderr and exit 1. Crates ship their own commands behind a `command`
feature (`route:list`, `queue:work`, `session:prune`, `event:list`).

## Features

| Feature | Adds |
| --- | --- |
| `action` | `command::action(a)`: an `Action` whose `Input: clap::Args` |
