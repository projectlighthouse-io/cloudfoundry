# password

Hash and verify passwords with Argon2id.

```toml
[dependencies]
password = { git = "https://github.com/projectlighthouse-io/cloudfoundry" }
```

```rust
let hash = password::hash("correct horse")?;
password::verify("correct horse", &hash)?;          // Err(Mismatch) on a wrong password
password::verify_none(input);                      // same cost when there is no user
password::needs_rehash(&hash);                     // true when made with older settings
```
