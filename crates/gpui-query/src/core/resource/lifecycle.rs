use crate::core::{
    QueryBeginResult, QueryFetchMode, QuerySignal, QueryStatus, QueryTimestamp, RequestGuard,
    RequestId, RequestPolicy, RequestSequencer, request::MaybeRequestId,
};

use super::QueryResource;

impl<T, E> QueryResource<T, E> {
    /// May short-circuit to `CacheHit` per the cache policy; replacing an
    /// in-flight request cancels its signal so the old fetcher can abort early.
    pub fn begin_request(
        &mut self,
        sequencer: &mut RequestSequencer,
        now_ms: u64,
        fetch_mode: QueryFetchMode,
    ) -> QueryBeginResult {
        self.begin_request_inner(now_ms, fetch_mode, MaybeRequestId::FromSequencer(sequencer))
    }

    /// `Some(id)` is used as-is (bucket-scoped ids from the hook layer);
    /// `None` falls back to the resource's own sequencer, which mints from a
    /// reserved scope so fallback ids never alias bucket-minted ones.
    pub fn begin_request_with_id(
        &mut self,
        maybe_request_id: Option<RequestId>,
        now_ms: u64,
        fetch_mode: QueryFetchMode,
    ) -> QueryBeginResult {
        self.begin_request_inner(
            now_ms,
            fetch_mode,
            MaybeRequestId::Provided(maybe_request_id),
        )
    }

    fn begin_request_inner(
        &mut self,
        now_ms: u64,
        fetch_mode: QueryFetchMode,
        mut id_source: MaybeRequestId<'_>,
    ) -> QueryBeginResult {
        // Early-return guards must not consume a sequence number.

        if fetch_mode == QueryFetchMode::Normal && self.should_short_circuit_cache(now_ms) {
            self.record_cache_hit();
            return QueryBeginResult::CacheHit;
        }

        // Checked before the IgnoreWhileLoading guard: stale data is always revalidated.
        if fetch_mode == QueryFetchMode::Normal && self.should_serve_stale_and_revalidate(now_ms) {
            self.record_cache_hit();

            if self.request_policy == RequestPolicy::IgnoreWhileLoading
                && let Some(active_request_id) = self.active_request_id
            {
                return QueryBeginResult::StaleCacheHit {
                    request_id: active_request_id,
                    status: self.status,
                    replaced_request_id: None,
                };
            }

            let replaced_request_id = self.active_request_id;
            if replaced_request_id.is_some() {
                self.cancelled_count = self.cancelled_count.saturating_add(1);
            }

            let request_id = id_source.next(&mut self.transient_sequencer);
            let status = self.begin_loading(request_id, now_ms);
            return QueryBeginResult::StaleCacheHit {
                request_id,
                status,
                replaced_request_id,
            };
        }

        if self.request_policy == RequestPolicy::IgnoreWhileLoading
            && let Some(active_request_id) = self.active_request_id
        {
            return QueryBeginResult::IgnoredWhileLoading { active_request_id };
        }

        let replaced_request_id = self.active_request_id;
        if replaced_request_id.is_some() {
            self.cancelled_count = self.cancelled_count.saturating_add(1);
        }

        let request_id = id_source.next(&mut self.transient_sequencer);
        let status = self.begin_loading(request_id, now_ms);
        QueryBeginResult::Started {
            request_id,
            status,
            replaced_request_id,
        }
    }

    /// A stale fetcher holding an old `RequestId` is rejected later by
    /// `accept_current_request()`.
    pub(crate) fn begin_loading(&mut self, request_id: RequestId, now_ms: u64) -> QueryStatus {
        let status = if self.has_data() {
            QueryStatus::LoadingWithData
        } else {
            QueryStatus::LoadingEmpty
        };
        self.status = status;
        self.active_request_id = Some(request_id);
        self.started_at = Some(QueryTimestamp::from(now_ms));
        self.error = None;

        if let Some(old_signal) = self.signal.as_ref() {
            old_signal.cancel();
        }
        self.signal = Some(QuerySignal::new());

        status
    }

    pub fn is_current_request(&self, request_id: RequestId) -> bool {
        self.active_request_id == Some(request_id)
    }

    /// `None` means the request was replaced or cancelled (counted as ignored).
    pub fn accept_current_request(&mut self, request_id: RequestId) -> Option<RequestGuard> {
        if self.is_current_request(request_id) {
            self.active_request_id = None;
            Some(RequestGuard::new(request_id))
        } else {
            self.mark_ignored_result();
            None
        }
    }

    /// Current data moves to `previous_data` before clearing (recoverable via
    /// `rollback_to_previous()`); cancelling a refetch does not destroy
    /// existing data, matching TanStack Query.
    pub fn cancel(&mut self, error: E) -> bool {
        if self.active_request_id.is_none() {
            return false;
        }

        self.active_request_id = None;
        self.status = QueryStatus::Cancelled;
        self.error = Some(error);
        self.cancelled_count = self.cancelled_count.saturating_add(1);

        if self.data.is_some() {
            self.previous_data = self.data.take();
            self.data_epoch = self.data_epoch.saturating_add(1);
        }

        if let Some(signal) = self.signal.as_ref() {
            signal.cancel();
        }

        true
    }

    pub fn mark_ignored_result(&mut self) {
        self.ignored_results = self.ignored_results.saturating_add(1);
    }

    /// Heuristic: data exists but the most recent fetch attempt failed, was
    /// cancelled, or is still revalidating in the background.
    pub fn is_data_stale(&self) -> bool {
        self.data.is_some()
            && matches!(
                self.status,
                QueryStatus::LoadingWithData | QueryStatus::Failure | QueryStatus::Cancelled
            )
    }

    /// Preserves the policies and key (configuration, not runtime state);
    /// counters are always reset, so read them first if needed.
    pub fn reset(&mut self) {
        if let Some(signal) = self.signal.as_ref() {
            signal.cancel();
        }
        self.status = QueryStatus::Idle;
        if self.data.is_some() {
            self.data_epoch = self.data_epoch.saturating_add(1);
        }
        self.data = None;
        self.error = None;
        self.active_request_id = None;
        self.started_at = None;
        self.last_updated_at = None;
        self.cache_hits = 0;
        self.cancelled_count = 0;
        self.ignored_results = 0;
        self.retry_count = 0;
        self.previous_data = None;
        self.signal = None;
    }

    /// Clears any stored error so `Success` keeps implying `error is None`,
    /// mirroring `apply_success`.
    pub fn rollback_to_previous(&mut self) -> bool {
        if let Some(prev) = self.previous_data.take() {
            self.data = Some(prev);
            self.status = QueryStatus::Success;
            self.error = None;
            self.data_epoch = self.data_epoch.saturating_add(1);
            return true;
        }
        false
    }

    pub fn set_data(&mut self, data: T) {
        self.previous_data = self.data.take();
        self.data = Some(data);
        self.data_epoch = self.data_epoch.saturating_add(1);
    }

    /// Status drops from `Success` to `Idle` so `Success` keeps implying data
    /// is available.
    pub fn clear_data(&mut self) {
        self.previous_data = self.data.take();
        self.data_epoch = self.data_epoch.saturating_add(1);
        if self.status == QueryStatus::Success {
            self.status = QueryStatus::Idle;
        }
    }
}
