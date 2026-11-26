# queue

Persist, claim, run and retry jobs.

```toml
[dependencies]
queue = { git = "https://github.com/projectlighthouse-io/cloudfoundry", features = ["postgres", "command"] }
```

```rust
#[derive(Serialize, Deserialize)]
struct ExecuteRun { run_id: i64 }

impl queue::Job for ExecuteRun {
    const NAME: &'static str = "execute_run";
    type Context = App;
    async fn run(self, app: &App) -> Result<(), queue::JobError> { Ok(()) }
}

let queue = queue::Queue::new(queue::PostgresStore::new(pool));
queue.push(ExecuteRun { run_id: 1 }).await?;

queue::Worker::new(queue, app).register::<ExecuteRun>().concurrency(8)
    .run(queue::shutdown_signal()).await;
```

At-least-once delivery: jobs must be idempotent. A heartbeat extends a running
job's hold, so long jobs are not claimed twice. Retries with backoff, then
`failed_jobs`. Tables: `art publish queue [--driver sqlite|mysql]`. See `examples/worker.rs`.

## Features

| Feature | Adds |
| --- | --- |
| `postgres`, `mysql`, `sqlite` | SQL stores (`MemoryStore` always) |
| `command` | `queue::command::{work, retry}` |
| `action` | `ActionJob`: `queue.push_action::<A>(input)`, `worker.register_action(\|app\| A { .. })` |
