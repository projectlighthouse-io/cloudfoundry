# configured

Load typed app config from files, with env vars and secrets resolved.

```toml
[dependencies]
configured = { git = "https://github.com/projectlighthouse-io/toolkit" }
```

```toml
# config/default.toml
app_name     = "env(APP_NAME, my-app)"
database_url = "required_env(DATABASE_URL)"
app_key      = "required_secret_env(APP_KEY)"

[session]
lifetime = "2h"
```

```rust
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub app_name: String,
    pub database_url: String,
    pub app_key: configured::SecretString,

    #[serde(default)]
    pub session: session::Config, // session feature `configured`
}

let Loaded { value: cfg, listing } = configured::load::<Config>("config")?;
println!("{}", listing.show()); // for config:show; secrets print as <redacted>
```

`load` reads `.env` (never overriding set vars), then `<dir>/default.{toml,yaml,yml,json}`
(required), then `<dir>/<APP_ENV>.*` (optional, overrides key by key).

| Form | Var unset or empty | Redacted |
| --- | --- | --- |
| `env(NAME)` | key left out: the field default applies | no |
| `env(NAME, fallback)` | `fallback` | no |
| `required_env(NAME)` | error | no |
| `secret_env(NAME)` | key left out | yes |
| `required_secret_env(NAME)` | error | yes |

- Every missing var and malformed form is reported at once, each with its config path.
- A malformed form (`env(X`, `required_env(X, d)`) is an error, never a literal.
- Env values fill typed fields (`u16`, `bool`); durations are `"90s"`, `"2h"`, `"14d"`.
- Secret values never appear in `listing.show()` (they print as `<redacted>`) or in error messages.
  Keys named `*_key`, `*_secret`, `*_password`, `*_token` and URL passwords are redacted too.
  `listing.show_secret_values()` prints the real values, for an explicit `--print-secret-values`.
- The listing has the keys set in files or env vars; omitted keys keep defaults and are not listed.
- No blanket environment source: only `env(...)` reads env vars.

For library crates: depend with `default-features = false` (no config-rs) and derive through
the re-export, with `configured::duration` / `configured::duration_or_never` for durations:

```rust
#[cfg_attr(
    feature = "configured",
    derive(configured::serde::Deserialize, configured::serde::Serialize),
    serde(crate = "configured::serde", default, deny_unknown_fields)
)]
pub struct Config {
    #[cfg_attr(feature = "configured", serde(with = "configured::duration"))]
    pub lifetime: Duration,
}
```

## Features

| Feature | Adds |
| --- | --- |
| `load` (default) | `configured::load`, with `config-rs` and `dotenvy` |

Docs: https://toolkit.projectlighthouse.io/docs/crates/configured/
