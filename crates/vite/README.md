# vite

Asset tags from a Vite manifest or dev server.

```toml
[dependencies]
vite = { git = "https://github.com/projectlighthouse-io/toolkit" }
```

```rust
let vite = vite::Vite::load("public", &["resources/js/app.ts"])?;   // a missing entry fails here
let head = vite.tags();            // dev server tags while `public/hot` exists, else the build's
let version = vite.manifest_hash();
```
