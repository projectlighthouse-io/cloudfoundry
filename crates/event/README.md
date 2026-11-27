# event

Dispatch typed events to listeners, in process.

```toml
[dependencies]
event = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["queue"] }
```

```rust
async fn update_status(app: App, e: RunFinished) -> Result<(), event::Error> { Ok(()) }

let events = event::Events::builder()
    .queue(&queue)
    .on(update_status)                         // inline, in order, first error stops
    .on_queued("notify_owner", notify_owner)   // a job; retried by the queue
    .build();

events.emit(&app, RunFinished { run_id: 1 }).await?;
```

## Features

| Feature | Adds |
| --- | --- |
| `queue` | `on_queued` and `register_jobs(worker)` |
| `action` | `on_action::<E, _>(a)`: an `Action` as a listener |
| `command` | `event::command::list` for `event:list` |
