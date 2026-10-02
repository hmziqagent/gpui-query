use std::collections::VecDeque;
use std::sync::Arc;

use super::{FetchDirection, InfiniteQueryResource};
use crate::core::{
    CachePolicy, QueryKey, QuerySignal, QueryStatus, QueryTimestamp, RequestId, RequestPolicy,
    RetryPolicy,
};

impl<T, E> InfiniteQueryResource<T, E> {
    /// On `Failure`, previously loaded pages remain present and valid; the
    /// failure applies only to the most recent page fetch.
    pub fn pages(&self) -> &VecDeque<Arc<T>> {
        &self.pages
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn first_page(&self) -> Option<&T> {
        self.pages.front().map(|a| a.as_ref())
    }

    pub fn last_page(&self) -> Option<&T> {
        self.pages.back().map(|a| a.as_ref())
    }

    pub fn first_page_arc(&self) -> Option<Arc<T>> {
        self.pages.front().cloned()
    }

    pub fn last_page_arc(&self) -> Option<Arc<T>> {
        self.pages.back().cloned()
    }

    pub fn has_next_page(&self) -> bool {
        self.has_next_page
    }

    pub fn has_previous_page(&self) -> bool {
        self.has_previous_page
    }

    pub fn is_fetching_next_page(&self) -> bool {
        matches!(
            self.fetching_direction,
            Some(super::lifecycle::PageDirection::Next)
        )
    }

    pub fn is_fetching_previous_page(&self) -> bool {
        matches!(
            self.fetching_direction,
            Some(super::lifecycle::PageDirection::Previous)
        )
    }

    pub fn max_pages(&self) -> Option<usize> {
        self.max_pages
    }

    pub fn direction(&self) -> FetchDirection {
        self.direction
    }

    pub fn status(&self) -> QueryStatus {
        self.status
    }

    pub fn error(&self) -> Option<&E> {
        self.error.as_ref()
    }

    pub fn is_loading(&self) -> bool {
        self.status.is_loading()
    }

    pub fn key(&self) -> &QueryKey {
        &self.key
    }

    pub fn active_request_id(&self) -> Option<RequestId> {
        self.active_request_id
    }

    pub fn cache_policy(&self) -> CachePolicy {
        self.cache_policy
    }

    pub fn request_policy(&self) -> RequestPolicy {
        self.request_policy
    }

    pub fn set_cache_policy(&mut self, policy: CachePolicy) {
        self.cache_policy = policy;
    }

    pub fn set_request_policy(&mut self, policy: RequestPolicy) {
        self.request_policy = policy;
    }

    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry_policy
    }

    /// Stored by `use_infinite_query` from its option so fetch helpers can
    /// read it back from the entity.
    pub fn set_retry_policy(&mut self, policy: RetryPolicy) {
        self.retry_policy = policy;
    }

    pub fn started_at_ms(&self) -> Option<u64> {
        self.started_at.map(QueryTimestamp::as_millis)
    }

    pub fn last_updated_at_ms(&self) -> Option<u64> {
        self.last_updated_at.map(QueryTimestamp::as_millis)
    }

    /// Counts page writes, not value changes; compare instead of
    /// deep-comparing page contents.
    pub fn data_epoch(&self) -> u64 {
        self.data_epoch
    }

    /// `None` when nothing was recorded or on clock skew (`checked_sub`).
    pub fn cache_age_ms(&self, now_ms: u64) -> Option<u64> {
        QueryTimestamp::from(now_ms).elapsed_since(self.last_updated_at?)
    }

    pub fn cache_hits(&self) -> u64 {
        self.cache_hits
    }

    pub fn cancelled_count(&self) -> u64 {
        self.cancelled_count
    }

    /// Bumped when a completion arrives with a stale request id, i.e. the
    /// fetch was replaced by a newer one before it finished.
    pub fn ignored_results(&self) -> u64 {
        self.ignored_results
    }

    pub fn retry_count(&self) -> u32 {
        self.retry_count
    }

    pub fn increment_retry(&mut self) {
        self.retry_count = self.retry_count.saturating_add(1);
    }

    /// Mirrors `QueryResource::mark_ignored_result` for the client layer's
    /// bulk-cancel path.
    pub fn mark_ignored_result(&mut self) {
        self.ignored_results = self.ignored_results.saturating_add(1);
    }

    pub fn reset_retry_count(&mut self) {
        self.retry_count = 0;
    }

    pub fn has_data(&self) -> bool {
        !self.pages.is_empty()
    }

    /// `Failure` returns `true` when pages were previously loaded: unlike
    /// `QueryResource`, where `Failure` invalidates the single data slot, the
    /// already-fetched pages stay valid.
    pub fn is_page_data_valid(&self) -> bool {
        match self.status {
            QueryStatus::Success | QueryStatus::LoadingWithData => true,
            QueryStatus::Failure => !self.pages.is_empty(),
            QueryStatus::LoadingEmpty | QueryStatus::Idle | QueryStatus::Cancelled => false,
        }
    }

    pub fn signal(&self) -> Option<&QuerySignal> {
        self.signal.as_ref()
    }
}
