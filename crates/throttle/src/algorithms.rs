//! The built-in algorithms. Each stores a few integers per key.

use std::time::Duration;

use crate::{Algorithm, Decision};

fn encode<const N: usize>(v: [i64; N]) -> Vec<u8> {
    v.iter().flat_map(|n| n.to_le_bytes()).collect()
}

/// `None` for missing state, or state another algorithm wrote (wrong length).
fn decode<const N: usize>(state: Option<&[u8]>) -> Option<[i64; N]> {
    let s = state.filter(|s| s.len() == N * 8)?;
    Some(std::array::from_fn(|i| i64::from_le_bytes(s[i * 8..i * 8 + 8].try_into().unwrap())))
}

fn ms(d: Duration) -> i64 {
    (d.as_millis() as i64).max(1)
}

fn denied(remaining: u64, retry_ms: f64) -> Decision {
    let retry = if retry_ms.is_finite() { Duration::from_millis(retry_ms.ceil().max(1.0) as u64) } else { Duration::MAX };
    Decision { allowed: false, remaining, retry_after: retry }
}

fn allowed(remaining: u64) -> Decision {
    Decision { allowed: true, remaining, retry_after: Duration::ZERO }
}

/// At most `max` hits per `window`, counted from a key's first hit. Simple and
/// cheap; allows up to `2 * max` across a window boundary.
#[derive(Clone, Copy, Debug)]
pub struct FixedWindow {
    max: u64,
    window: i64,
}

impl FixedWindow {
    pub fn new(max: u64, window: Duration) -> Self {
        Self { max, window: ms(window) }
    }
}

impl Algorithm for FixedWindow {
    fn decide(&self, state: Option<&[u8]>, now: i64, cost: u64) -> (Decision, Option<(Vec<u8>, i64)>) {
        let [start, count] = decode(state).filter(|[start, _]| now < start + self.window).unwrap_or([now, 0]);
        let count = count as u64 + cost;
        if count > self.max {
            return (denied(self.max.saturating_sub(count - cost), (start + self.window - now) as f64), None);
        }
        (allowed(self.max - count), Some((encode([start, count as i64]), start + self.window)))
    }
}

/// At most `max` hits in any `window`-long span, estimated from the current and
/// previous fixed windows (the previous one weighted by how much of it still
/// overlaps). Smooth at window boundaries; stores three integers.
#[derive(Clone, Copy, Debug)]
pub struct SlidingWindow {
    max: u64,
    window: i64,
}

impl SlidingWindow {
    pub fn new(max: u64, window: Duration) -> Self {
        Self { max, window: ms(window) }
    }
}

impl Algorithm for SlidingWindow {
    fn decide(&self, state: Option<&[u8]>, now: i64, cost: u64) -> (Decision, Option<(Vec<u8>, i64)>) {
        let w = self.window;
        let cur_start = now - now.rem_euclid(w);
        let (prev, cur) = match decode(state) {
            Some([start, cur, _]) if start == cur_start - w => (cur, 0),
            Some([start, cur, prev]) if start == cur_start => (prev, cur),
            _ => (0, 0),
        };
        let (prev, cur, cost, max, wf) = (prev as f64, cur as f64, cost as f64, self.max as f64, w as f64);
        let elapsed = (now - cur_start) as f64;
        let estimate = prev * (1.0 - elapsed / wf) + cur;

        if estimate + cost > max {
            // When does prev * (1 - t/w) + cur + cost fit under max?
            let room = max - cur - cost;
            let retry = if room >= 0.0 {
                wf * (1.0 - room / prev) - elapsed
            } else {
                // Not in this window; in the next one this window's count is the weighted one.
                let room = max - cost;
                let t = if room < 0.0 { f64::INFINITY } else if cur == 0.0 { 0.0 } else { (wf * (1.0 - room / cur)).max(0.0) };
                wf - elapsed + t
            };
            return (denied((max - estimate).max(0.0) as u64, retry), None);
        }
        let new = [cur_start, (cur + cost) as i64, prev as i64];
        (allowed((max - estimate - cost) as u64), Some((encode(new), cur_start + 2 * w)))
    }
}

/// A bucket of `capacity` tokens, refilled by `refill` tokens every `every`.
/// Each hit takes one. Allows bursts up to `capacity`, then a steady rate. The
/// usual choice for calls to an external API.
#[derive(Clone, Copy, Debug)]
pub struct TokenBucket {
    capacity: f64,
    /// Tokens per millisecond.
    rate: f64,
}

impl TokenBucket {
    pub fn new(capacity: u64, refill: u64, every: Duration) -> Self {
        Self { capacity: capacity as f64, rate: refill as f64 / ms(every) as f64 }
    }
}

impl Algorithm for TokenBucket {
    fn decide(&self, state: Option<&[u8]>, now: i64, cost: u64) -> (Decision, Option<(Vec<u8>, i64)>) {
        let tokens = match decode(state) {
            Some([bits, last]) => (f64::from_bits(bits as u64) + (now - last).max(0) as f64 * self.rate).min(self.capacity),
            None => self.capacity,
        };
        let cost = cost as f64;
        if cost > tokens {
            // A cost above capacity never fits: retry is infinite.
            let retry = if cost > self.capacity { f64::INFINITY } else { (cost - tokens) / self.rate };
            return (denied(tokens as u64, retry), None);
        }
        let left = tokens - cost;
        // Once refilled to capacity the state is the same as none at all.
        let full_at = now + ((self.capacity - left) / self.rate).ceil() as i64 + 1;
        (allowed(left as u64), Some((encode([left.to_bits() as i64, now]), full_at)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run hits at the given times through `a`, keeping state like a store would.
    fn run(a: &impl Algorithm, times: &[i64]) -> Vec<Decision> {
        let mut state: Option<(Vec<u8>, i64)> = None;
        times
            .iter()
            .map(|&now| {
                let current = state.as_ref().filter(|(_, exp)| *exp > now).map(|(s, _)| s.as_slice());
                let (d, new) = a.decide(current, now, 1);
                if new.is_some() {
                    state = new;
                }
                d
            })
            .collect()
    }

    fn allowed(ds: &[Decision]) -> Vec<bool> {
        ds.iter().map(|d| d.allowed).collect()
    }

    #[test]
    fn fixed_window() {
        let a = FixedWindow::new(2, Duration::from_millis(1000));
        let ds = run(&a, &[0, 100, 200, 999, 1000, 1001]);
        assert_eq!(allowed(&ds), [true, true, false, false, true, true]);
        assert_eq!(ds[2].retry_after, Duration::from_millis(800));
        assert_eq!(ds[0].remaining, 1);
    }

    #[test]
    fn sliding_window_weights_the_previous_window() {
        let a = SlidingWindow::new(4, Duration::from_millis(1000));
        // 4 hits late in window 0, then window 1: at 1250 the previous window still weighs 0.75 * 4 = 3.
        let ds = run(&a, &[900, 910, 920, 930, 940, 1250, 1260, 1500, 1510]);
        assert_eq!(allowed(&ds), [true, true, true, true, false, true, false, true, false]);
        // At 1260: 0.74 * 4 + 1 = 3.96, plus 1 > 4. Fits once 4 * (1 - t) + 1 + 1 <= 4: t = 0.5.
        assert_eq!(ds[6].retry_after, Duration::from_millis(240));
        // Two windows later nothing is left.
        assert!(run(&a, &[0, 0, 0, 0, 2000]).last().unwrap().allowed);
    }

    #[test]
    fn token_bucket_bursts_then_refills() {
        let a = TokenBucket::new(3, 1, Duration::from_millis(100));
        let ds = run(&a, &[0, 0, 0, 0, 50, 100, 100]);
        assert_eq!(allowed(&ds), [true, true, true, false, false, true, false]);
        assert_eq!(ds[3].retry_after, Duration::from_millis(100));
        assert_eq!(ds[4].retry_after, Duration::from_millis(50));
        let (d, _) = a.decide(None, 0, 4);
        assert!(!d.allowed && d.retry_after == Duration::MAX, "cost above capacity never fits");
    }

    #[test]
    fn foreign_state_is_ignored() {
        let (d, _) = FixedWindow::new(1, Duration::from_secs(1)).decide(Some(&[1, 2, 3]), 0, 1);
        assert!(d.allowed);
    }
}
