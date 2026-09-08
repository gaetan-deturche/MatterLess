use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// Client-side token bucket.
///
/// Phase 0 found this server returns no `X-RateLimit-*` headers at all, so the
/// limiter cannot adapt from responses and has to be conservative up front. The
/// server default is roughly 10 requests/second per session; we sit under it and
/// still allow a burst for the fan-out when a channel opens.
pub struct RateLimiter {
    tokens_per_second: f64,
    burst: f64,
    state: Mutex<BucketState>,
}

struct BucketState {
    /// Available permits. Goes negative while callers are queued, which is what
    /// reserves their slot in arrival order.
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn per_second(rate: u32, burst: u32) -> Self {
        let tokens_per_second = f64::from(rate.max(1));
        let burst = f64::from(burst.max(1));
        Self {
            tokens_per_second,
            burst,
            state: Mutex::new(BucketState {
                tokens: burst,
                last_refill: Instant::now(),
            }),
        }
    }

    /// Conservative default: 8/s sustained with a burst of 16.
    pub fn conservative() -> Self {
        Self::per_second(8, 16)
    }

    /// Resolves when the caller may issue its request.
    pub async fn acquire(&self) {
        let wait = {
            let mut state = self.state.lock().await;
            let now = Instant::now();
            let elapsed = now
                .saturating_duration_since(state.last_refill)
                .as_secs_f64();
            state.tokens = (state.tokens + elapsed * self.tokens_per_second).min(self.burst);
            state.last_refill = now;

            state.tokens -= 1.0;
            if state.tokens >= 0.0 {
                None
            } else {
                Some(Duration::from_secs_f64(
                    -state.tokens / self.tokens_per_second,
                ))
            }
        };

        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn burst_is_immediate_then_throttled() {
        let limiter = RateLimiter::per_second(10, 4);
        let started = Instant::now();
        for _ in 0..4 {
            limiter.acquire().await;
        }
        assert_eq!(started.elapsed(), Duration::ZERO, "burst must not sleep");

        limiter.acquire().await;
        assert!(
            started.elapsed() >= Duration::from_millis(100),
            "past the burst the limiter must space requests, waited {:?}",
            started.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sustained_rate_is_respected() {
        let limiter = RateLimiter::per_second(10, 1);
        let started = Instant::now();
        for _ in 0..11 {
            limiter.acquire().await;
        }
        // One burst token plus ten spaced at 100 ms.
        assert!(
            started.elapsed() >= Duration::from_secs(1),
            "ten requests past the burst must take a second, took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn idle_time_refills_the_burst() {
        let limiter = RateLimiter::per_second(10, 4);
        for _ in 0..4 {
            limiter.acquire().await;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;

        let resumed = Instant::now();
        for _ in 0..4 {
            limiter.acquire().await;
        }
        assert_eq!(
            resumed.elapsed(),
            Duration::ZERO,
            "the bucket should have refilled to full while idle"
        );
    }
}
