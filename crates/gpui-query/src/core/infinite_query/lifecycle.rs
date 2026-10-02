use std::sync::Arc;

use crate::core::{
    QuerySignal, QueryStatus, QueryTimestamp, RequestGuard, RequestId, RequestPolicy,
    RequestSequencer, request::MaybeRequestId,
};

use super::FetchDirection;
use super::InfiniteQueryResource;

/// Backs the serde-serialized `fetching_direction` field; a single
/// `Option<PageDirection>` makes the one-direction-in-flight invariant hold
/// by construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum PageDirection {
    Next,
    Previous,
}

impl<T, E> InfiniteQueryResource<T, E> {
    /// Under `LatestWins` this replaces an in-flight request in either
    /// direction; `IgnoreWhileLoading` only guards within the same direction.
    pub fn begin_fetch_next(
        &mut self,
        sequencer: &mut RequestSequencer,
        now_ms: u64,
    ) -> Option<RequestId> {
        self.begin_fetch(
            PageDirection::Next,
            MaybeRequestId::FromSequencer(sequencer),
            now_ms,
        )
    }

    /// Mirror of [`begin_fetch_next`](Self::begin_fetch_next) for the
    /// previous direction.
    pub fn begin_fetch_previous(
        &mut self,
        sequencer: &mut RequestSequencer,
        now_ms: u64,
    ) -> Option<RequestId> {
        self.begin_fetch(
            PageDirection::Previous,
            MaybeRequestId::FromSequencer(sequencer),
            now_ms,
        )
    }

    /// `Some(id)` is used as-is, matching ids the bucket pre-allocated via
    /// `QueryClient::next_request_id_for_infinite_key`; `None` falls back to
    /// the resource's own sequencer.
    pub fn begin_fetch_next_with_id(
        &mut self,
        maybe_request_id: Option<RequestId>,
        now_ms: u64,
    ) -> Option<RequestId> {
        self.begin_fetch(
            PageDirection::Next,
            MaybeRequestId::Provided(maybe_request_id),
            now_ms,
        )
    }

    /// See [`begin_fetch_next_with_id`](Self::begin_fetch_next_with_id).
    pub fn begin_fetch_previous_with_id(
        &mut self,
        maybe_request_id: Option<RequestId>,
        now_ms: u64,
    ) -> Option<RequestId> {
        self.begin_fetch(
            PageDirection::Previous,
            MaybeRequestId::Provided(maybe_request_id),
            now_ms,
        )
    }

    fn begin_fetch(
        &mut self,
        direction: PageDirection,
        mut id_source: MaybeRequestId<'_>,
        now_ms: u64,
    ) -> Option<RequestId> {
        let (has_page, is_fetching_same_direction) = match direction {
            PageDirection::Next => (self.has_next_page, self.is_fetching_next_page()),
            PageDirection::Previous => (self.has_previous_page, self.is_fetching_previous_page()),
        };

        if !has_page {
            return None;
        }

        if is_fetching_same_direction && self.request_policy == RequestPolicy::IgnoreWhileLoading {
            return None;
        }

        if self.active_request_id.is_some() {
            self.cancelled_count = self.cancelled_count.saturating_add(1);
        }

        if let Some(old_signal) = self.signal.as_ref() {
            old_signal.cancel();
        }

        self.fetching_direction = Some(direction);

        let request_id = id_source.next(&mut self.transient_sequencer);
        self.active_request_id = Some(request_id);
        self.status = if self.pages.is_empty() {
            QueryStatus::LoadingEmpty
        } else {
            QueryStatus::LoadingWithData
        };
        self.started_at = Some(QueryTimestamp::from(now_ms));
        self.error = None;
        self.signal = Some(QuerySignal::new());

        Some(request_id)
    }

    /// `None` when the request was replaced or cancelled; stale completions
    /// bump `ignored_results`.
    pub fn accept_current_request(&mut self, request_id: RequestId) -> Option<RequestGuard> {
        if self.is_current_request(request_id) {
            self.active_request_id = None;
            Some(RequestGuard::new(request_id))
        } else {
            self.mark_ignored_result();
            None
        }
    }

    /// Pages evicted by the `max_pages` bound are dropped here;
    /// `append_page`/`prepend_page` are the variants that return them.
    pub fn complete_success_with_guard(
        &mut self,
        _guard: RequestGuard,
        page: T,
        has_more: bool,
        is_next: bool,
        now_ms: u64,
    ) {
        if is_next {
            self.pages.push_back(Arc::new(page));
            self.has_next_page = has_more;
            self.enforce_max_pages_remove_front();
        } else {
            self.pages.push_front(Arc::new(page));
            self.has_previous_page = has_more;
            self.enforce_max_pages_remove_back();
        }
        self.data_epoch = self.data_epoch.saturating_add(1);

        self.status = QueryStatus::Success;
        self.error = None;
        self.last_updated_at = Some(QueryTimestamp::from(now_ms));
        self.fetching_direction = None;
        self.signal = None;
    }

    /// Previously loaded pages are NOT cleared; see
    /// [`is_page_data_valid`](Self::is_page_data_valid).
    pub fn complete_failure_with_guard(&mut self, _guard: RequestGuard, error: E) {
        self.status = QueryStatus::Failure;
        self.error = Some(error);
        self.fetching_direction = None;
        self.signal = None;
    }

    /// Accept-and-complete in one call; evicted pages are dropped.
    pub fn complete_page_success(
        &mut self,
        request_id: RequestId,
        page: T,
        has_more: bool,
        is_next: bool,
        now_ms: u64,
    ) -> bool {
        if let Some(guard) = self.accept_current_request(request_id) {
            self.complete_success_with_guard(guard, page, has_more, is_next, now_ms);
            true
        } else {
            false
        }
    }

    /// Accept-and-complete in one call; loaded pages are NOT cleared.
    pub fn complete_page_failure(&mut self, request_id: RequestId, error: E) -> bool {
        if let Some(guard) = self.accept_current_request(request_id) {
            self.complete_failure_with_guard(guard, error);
            true
        } else {
            false
        }
    }

    pub fn is_current_request(&self, request_id: RequestId) -> bool {
        self.active_request_id == Some(request_id)
    }

    /// `max_pages` and `direction` persist; the flags reset to the current
    /// direction's defaults, so re-set them after reset if the query was
    /// previously exhausted.
    pub fn reset(&mut self) {
        if let Some(signal) = self.signal.as_ref() {
            signal.cancel();
        }
        if !self.pages.is_empty() {
            self.data_epoch = self.data_epoch.saturating_add(1);
        }
        self.pages.clear();
        self.status = QueryStatus::Idle;
        self.error = None;
        self.active_request_id = None;
        self.started_at = None;
        self.last_updated_at = None;
        self.cache_hits = 0;
        self.cancelled_count = 0;
        self.ignored_results = 0;
        self.retry_count = 0;
        let (has_next, has_prev) = match self.direction {
            FetchDirection::ForwardOnly => (true, false),
            FetchDirection::Bidirectional => (false, false),
        };
        self.has_next_page = has_next;
        self.has_previous_page = has_prev;
        self.fetching_direction = None;
        self.signal = None;
    }

    pub fn invalidate(&mut self) {
        self.last_updated_at = None;
    }
}
