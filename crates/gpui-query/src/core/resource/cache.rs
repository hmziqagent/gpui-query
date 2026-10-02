use crate::core::{QueryStatus, QueryTimestamp};

use super::QueryResource;

impl<T, E> QueryResource<T, E> {
    pub fn cache_age_ms(&self, now_ms: u64) -> Option<u64> {
        QueryTimestamp::from(now_ms).elapsed_since(self.last_updated_at?)
    }

    /// The TTL boundary is inclusive (`age <= ttl`), unlike HTTP `max-age`;
    /// the stale-while-revalidate window is not fresh, it is stale-but-serveable.
    pub fn is_cache_fresh(&self, now_ms: u64) -> bool {
        self.has_data()
            && self
                .cache_age_ms(now_ms)
                .is_some_and(|age_ms| self.cache_policy.is_fresh(age_ms))
    }

    pub fn is_stale_but_serveable(&self, now_ms: u64) -> bool {
        self.has_data()
            && self
                .cache_age_ms(now_ms)
                .is_some_and(|age_ms| self.cache_policy.is_stale_but_serveable(age_ms))
    }

    pub fn is_cache_expired(&self, now_ms: u64) -> bool {
        !self.has_data()
            || self
                .cache_age_ms(now_ms)
                .is_none_or(|age_ms| self.cache_policy.is_expired(age_ms))
    }

    pub fn should_short_circuit_cache(&self, now_ms: u64) -> bool {
        self.cache_policy.can_short_circuit() && self.is_cache_fresh(now_ms)
    }

    /// When `true`, the caller serves the stale data immediately and starts a
    /// background fetch to revalidate.
    pub fn should_serve_stale_and_revalidate(&self, now_ms: u64) -> bool {
        self.cache_policy.can_serve_stale() && self.is_stale_but_serveable(now_ms)
    }

    /// Skips the `Success` transition when in `Failure`/`Cancelled`: a hit on
    /// old data must not clear an error the consumer is already handling.
    pub(crate) fn record_cache_hit(&mut self) {
        self.cache_hits = self.cache_hits.saturating_add(1);
        if !matches!(self.status, QueryStatus::Failure | QueryStatus::Cancelled) {
            self.status = QueryStatus::Success;
            self.error = None;
        }
    }

    /// Data is retained; only the last-updated timestamp is cleared.
    pub fn invalidate(&mut self) {
        self.last_updated_at = None;
    }
}
