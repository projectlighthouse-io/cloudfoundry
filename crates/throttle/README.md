# throttle

Rate limit anything per key: requests, logins, calls to an external API, a function.

```toml
[dependencies]
throttle = { git = "https://github.com/projectlighthouse-io/cloudfoundry" }
```

```rust
use throttle::{FixedWindow, Limiter, SlidingWindow, TokenBucket};

let logins = Limiter::memory(FixedWindow::new(5, Duration::from_secs(60)));
let d = logins.hit("ada@example.com").await?;   // d.allowed, d.remaining, d.retry_after

let api = Limiter::memory(SlidingWindow::new(100, Duration::from_secs(60)));
let github = Limiter::memory(TokenBucket::new(10, 1, Duration::from_secs(1)));
github.wait("github").await?;                    // sleep until allowed, then go
```

| Algorithm | Params | Behaviour |
| --- | --- | --- |
| `FixedWindow` | `max`, `window` | Window starts at the first hit. Cheapest; up to `2 * max` across a boundary |
| `SlidingWindow` | `max`, `window` | Any `window`-long span, estimated from two windows. Smooth |
| `TokenBucket` | `capacity`, `refill`, `every` | Bursts up to `capacity`, then a steady rate |

`hit` counts (denied hits are not counted), `check` peeks without counting,
`clear` resets a key, `wait` sleeps until a hit is allowed. Implement
`Algorithm` for your own; implement `Store` to keep counts elsewhere.
`MemoryStore` is per process. To share limits across servers, enable `postgres`, `sqlite`
or `mysql`, run `art publish throttle [--driver sqlite|mysql]` and `art migrate`, then
`Limiter::new(throttle::PostgresStore::new(pool), algorithm)`. Feature `command` adds
`throttle::command::prune` for `throttle:prune`.
