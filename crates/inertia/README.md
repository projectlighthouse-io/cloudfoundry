# inertia

Speak the Inertia.js v3 protocol.

```toml
[dependencies]
inertia = { git = "https://github.com/ProjectLighthouse/cloudfoundry", features = ["validate"] }
```

```rust
#[derive(serde::Serialize)]
struct RunShow { run: RunView, logs: inertia::Optional<Vec<Line>> }

async fn show(visit: inertia::Visit, Path(id): Path<i64>) -> inertia::Page<RunShow> {
    inertia::render("runs/show", RunShow {
        run: load(id).await,
        logs: visit.optional("logs", load_logs(id)).await,   // only when a partial reload asks
    })
}

router.layer(inertia::layer()
    .root_template("resources/views/app.html")
    .version(vite.manifest_hash())
    .head(move || vite.tags()));
```

HTML on first visits, JSON on Inertia visits, partial reloads, `Optional`/`Defer`
props, the asset version check (409) and 303s after PUT/PATCH/DELETE.

## Features

| Feature | Adds |
| --- | --- |
| `validate` | On an Inertia visit, a validation failure redirects back with `props.errors` |
