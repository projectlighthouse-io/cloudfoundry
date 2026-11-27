# art

The CLI: scaffolds apps, runs the dev loop, forwards everything else to the app
binary (`cargo run -- <command>`). Depends on none of the crates.

```bash
cargo install --git https://github.com/projectlighthouse-io/cloudfoundry art
art new myapp --git https://github.com/projectlighthouse-io/cloudfoundry
cd myapp && npm install
art publish session && art publish queue && art migrate
art dev
```

| Command | Does |
| --- | --- |
| `new <name> [--driver sqlite\|mysql]` | App tree, `main.rs` wiring, `.env` with a fresh `APP_KEY`, Vue + Inertia frontend (with SSR entry) |
| `dev` | Runs the app (restart on change), Vite, and TypeScript prop types export |
| `build` | Type export, Vite production build, `cargo build --release` |
| `migrate`, `migrate:rollback`, `make:migration` | Through `sqlx-cli` |
| `make:handler`, `make:action`, `make:request`, `make:response` | One file plus one `mod` line |
| `make:page <path>` | Props struct, Vue page, and its TypeScript type |
| `make:middleware`, `make:job`, `make:event [--listener]`, `make:command` | Stubs |
| `publish <package> [--driver sqlite\|mysql] [--with-ui]` | Copy a package's stubs into the app; `--with-ui` adds its Inertia pages (`--list`, `--force`) |
| `key:generate` | A fresh `APP_KEY` |
| `ssr` | Run the Inertia SSR server from `art build`; set `INERTIA_SSR_URL=http://127.0.0.1:13714` |
| anything else | `cargo run -- …`: `route:list`, `queue:work`, `event:list`, `config:show`, … |

Generators never rewrite existing code; they print the lines to add.
