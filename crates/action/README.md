# action

One trait for a unit of application logic.

```toml
[dependencies]
action = { git = "https://github.com/ProjectLighthouse/cloudfoundry" }
```

```rust
use action::Action;

struct StartRun { db: Db<Primary> }

impl Action for StartRun {
    type Input = StartRunInput;
    type Output = Result<Run, AppError>;

    async fn run(&self, input: StartRunInput) -> Result<Run, AppError> { /* ... */ }
}
```

Run it directly, as a handler (`route::action`), a command (`command::action`) or an
event listener (`event` feature `action`). No dependencies.
