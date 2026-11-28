# crypt

Encrypt values at rest: XChaCha20-Poly1305, keys derived from `APP_KEY`. `APP_KEY="new,old"` encrypts with the
first and opens with any, so keys rotate.

```toml
[dependencies]
crypt = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["postgres", "command"] }
```

```rust
let crypt = crypt::Encrypter::new(crypt::Keys::parse(app_key)?);
let sealed = crypt.encrypt(b"factor2/totp:42", secret)?;          // bound to its purpose

// A serde value in a bytea/blob column (features sqlx or a driver). Debug prints `…`.
let creds = crypt::Encrypted::seal(&crypt, b"integrations:7", &credentials)?;
let credentials: Credentials = creds.open(&crypt, b"integrations:7")?;
```

`crypt:rotate --table t --column c [--id id] [--purpose 'p:{id}'] [--dry-run]` re-encrypts a column with the
newest key (pages by id, conditional updates, idempotent); register it with `crypt::command::rotate(|app| ...)`.
`crypt::rotate::Rotate` is implemented for the sqlx pools of the enabled drivers.

Tests: `cargo test -p crypt --features sqlite,command`; Postgres and MySQL with `CRYPT_PG_URL` / `CRYPT_MYSQL_URL`.

Docs: https://toolkit.projectlighthouse.io/docs/crates/crypt.md
