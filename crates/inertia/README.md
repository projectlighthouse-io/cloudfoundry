# inertia

Speak the Inertia.js v3 protocol.

```toml
[dependencies]
inertia = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["validate"] }
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

HTML on first visits, JSON on Inertia visits, partial reloads, `Optional`/`Defer`/
`Merge`/`Once` props, error bags, the asset version check (409) and 303s after
PUT/PATCH/DELETE. `.pages_dir(dir)` catches a mistyped component in debug builds.

## Config

`.config(cfg)?` sets the root template, pages dir, history encryption and SSR, and returns
`Err(ConfigError)` for an unreadable template, one without `{{ inertia }}`, a non-`http://`
SSR URL, or `ssr_url` without feature `ssr`. Empty strings mean unset. Version, `head` and
`share` stay methods.

```toml
[inertia]
root_template = "resources/views/app.html"
ssr_url = "http://127.0.0.1:13714"
ssr_timeout = "2s"
```

## Features

| Feature | Adds |
| --- | --- |
| `validate` | On an Inertia visit, a validation failure redirects back with `props.errors` |
| `ssr` | `.ssr("http://127.0.0.1:13714")`: render first visits on the Inertia SSR server, falling back to the client |
| `configured` | `inertia::Config` deserializes (durations as `"2s"`) |
