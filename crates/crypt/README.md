# crypt

Encrypt values at rest (database columns, files), with key rotation.

```toml
[dependencies]
crypt = { git = "https://github.com/projectlighthouse-io/toolkit" }
```

```rust
use crypt::{Encrypter, Keys};

let crypt = Encrypter::new(Keys::from_env("APP_KEY")?);      // comma-separated, newest first
let sealed = crypt.encrypt(b"totp:42", secret)?;             // store these bytes (bytea / blob)
let secret = crypt.decrypt(b"totp:42", &sealed)?;
if crypt.needs_rotation(&sealed) { /* re-encrypt with the newest key and store */ }
```

XChaCha20-Poly1305 with a random 24-byte nonce. The `purpose` (e.g. `totp:<user id>`) is bound into the tag, so
a value copied to another row or column does not decrypt. The key is derived from `APP_KEY` with its own label,
so the same `APP_KEY` can serve `securecookie` without the two ever sharing a key. Every failure is one opaque
error; keys never print. No timestamps: values at rest do not expire.

Format: `version (1) | key id (4) | nonce (24) | ciphertext + tag`.
