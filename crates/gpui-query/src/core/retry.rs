use serde::{Deserialize, Serialize};

/// Defaults: 3 retries, exponential backoff, 1s base delay, 30s cap.
///
/// # Examples
///
/// ```
/// use gpui_query::RetryPolicy;
///
/// let policy = RetryPolicy::new(5)
///     .with_delay(500)
///     .with_exponential_backoff()
///     .with_max_delay(10_000);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// 0 = no retries.
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub exponential_backoff: bool,
    pub max_retry_delay_ms: u64,
}

impl RetryPolicy {
    pub const fn no_retries() -> Self {
        Self {
            max_retries: 0,
            retry_delay_ms: 0,
            exponential_backoff: false,
            max_retry_delay_ms: 0,
        }
    }

    pub const fn new(max_retries: u32) -> Self {
        Self {
            max_retries,
            retry_delay_ms: 1000,
            exponential_backoff: false,
            max_retry_delay_ms: 30_000,
        }
    }

    pub fn with_delay(mut self, delay_ms: u64) -> Self {
        self.retry_delay_ms = delay_ms;
        self
    }

    pub fn with_exponential_backoff(mut self) -> Self {
        self.exponential_backoff = true;
        self
    }

    pub fn with_max_delay(mut self, max_ms: u64) -> Self {
        self.max_retry_delay_ms = max_ms;
        self
    }

    /// Absolute ceiling for any single retry delay (1 hour).
    const ABSOLUTE_MAX_DELAY_MS: u64 = 3_600_000;

    /// With exponential backoff: `retry_delay_ms * 2^attempt`, capped by
    /// `max_retry_delay_ms` and then a 1-hour hard ceiling.
    pub fn delay_for_attempt(&self, attempt: u32) -> u64 {
        if !self.exponential_backoff {
            return self.retry_delay_ms;
        }
        let shift = attempt.min(62);
        let factor = 1u64 << shift;
        let delay = self.retry_delay_ms.saturating_mul(factor);
        delay
            .min(self.max_retry_delay_ms)
            .min(Self::ABSOLUTE_MAX_DELAY_MS)
    }

    pub fn should_retry(&self, current_retries: u32) -> bool {
        current_retries < self.max_retries
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::new(3).with_exponential_backoff()
    }
}
