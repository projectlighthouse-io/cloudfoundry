# csrf

Issue and verify CSRF tokens.

```toml
[dependencies]
csrf = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["route"] }
```

```rust
router
    .layer(session::layer(store, codec))                   // provides the token store
    .layer(csrf::layer::<session::Session>().except(["webhooks.github"]));
```

Unsafe methods must send the token in `X-XSRF-TOKEN` (the Inertia v3 default, read
from the `XSRF-TOKEN` cookie this layer sets), `X-CSRF-TOKEN`, or a `_token` form field. Failure is a bare 419.
Constant-time comparison.

## Features

| Feature | Adds |
| --- | --- |
| `route` | `except([...])`: exempt routes by name |
