# art

The CLI: scaffolds apps, runs the dev loop, forwards everything else to the app
binary (`cargo run -- <command>`). Depends on none of the crates.

```bash
cargo install --git https://github.com/ProjectLighthouse/cloudfoundry art
art new myapp --git https://github.com/ProjectLighthouse/cloudfoundry
cd myapp && npm install
art publish session && art publish queue && art migrate
art dev
```

| Command | Does |
| --- | --- |
| `new <name>` | App tree, `main.rs` wiring, `.env` with a fresh `APP_KEY`, Vue + Inertia frontend |
| `dev` | Runs the app (restart on change), Vite, and TypeScript prop types export |
| `build` | Type export, Vite production build, `cargo build --release` |
| `migrate`, `migrate:rollback`, `make:migration` | Through `sqlx-cli` |
| `make:handler`, `make:action`, `make:request`, `make:response` | One file plus one `mod` line |
| `make:page <path>` | Props struct, Vue page, and its TypeScript type |
| `make:middleware`, `make:job`, `make:event [--listener]`, `make:command` | Stubs |
| `publish <package> [--driver sqlite\|mysql]` | Copy a package's stubs into the app (`--list`, `--force`) |
| `key:generate` | A fresh `APP_KEY` |
| anything else | `cargo run -- …`: `route:list`, `queue:work`, `event:list`, `config:show`, … |

Generators never rewrite existing code; they print the lines to add.
