# foundry

Small, independent Rust crates for server-driven web apps, and `art`, a CLI to
scaffold and run them. Each crate stands alone; use only what you need.

| Crate | Does |
| --- | --- |
| `route` | Match a request to a handler. Named routes, groups, tower layers. Features: `server`, `command` |
| `db` | Typed `sqlx` pool handles and explicit transactions. Features: `postgres`, `mysql`, `sqlite`, `singleton` |
| `command` | Named commands in the app binary (`./app serve`, `./app route:list`) |

Crates ship as git dependencies:

```toml
route = { git = "<this repo>", features = ["server"] }
```

The CLI:

```bash
cargo install --git <this repo> art
art new myapp --git <this repo>
```
