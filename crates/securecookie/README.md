# securecookie

Sign and encrypt cookie values.

```toml
[dependencies]
securecookie = { git = "https://github.com/ProjectLighthouse/cloudfoundry" }
```

```rust
use securecookie::{Codec, Keys};

let codec = Codec::new(Keys::from_env("APP_KEY")?);   // comma-separated, newest first
let value = codec.encrypt("session", &session_id)?;
let id: String = codec.decrypt("session", &value, max_age)?;
```

XChaCha20-Poly1305 for encryption, HMAC-SHA256 for signing. The cookie name is
bound into the tag. Every failure is the same opaque error. Keys never print.
