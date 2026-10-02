//! Infinite query resource for paginated data. Pages live in a
//! `VecDeque<Arc<T>>` (O(1) append/prepend); `max_pages` (default 50) evicts
//! from the opposite side, and `set_max_pages(Some(0))` means unbounded.

mod accessors;
mod lifecycle;
mod page_management;
mod resource;

pub use resource::{FetchDirection, InfiniteQueryResource};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CachePolicy, QueryKey, QueryStatus, RequestPolicy, RequestSequencer};

    fn make_resource() -> InfiniteQueryResource<Vec<String>> {
        InfiniteQueryResource::new(
            QueryKey::from("items"),
            CachePolicy::Ttl { ttl_ms: 60_000 },
            RequestPolicy::LatestWins,
        )
    }

    fn make_bidirectional_resource() -> InfiniteQueryResource<Vec<String>> {
        InfiniteQueryResource::new_bidirectional(
            QueryKey::from("items"),
            CachePolicy::Ttl { ttl_ms: 60_000 },
            RequestPolicy::LatestWins,
        )
    }

    #[test]
    fn new_resource_is_idle() {
        let r = make_resource();
        assert_eq!(r.status(), QueryStatus::Idle);
        assert!(r.pages().is_empty());
        assert_eq!(r.max_pages(), Some(50));
    }

    #[test]
    fn begin_fetch_next_returns_request_id() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();
        let id = r.begin_fetch_next(&mut seq, 1_000);
        assert!(id.is_some());
        assert!(r.is_fetching_next_page());
    }

    #[test]
    fn complete_page_success_appends() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();
        let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        let accepted = r.complete_page_success(id, vec!["a".to_string()], true, true, 2_000);
        assert!(accepted);
        assert_eq!(r.page_count(), 1);
        assert_eq!(r.status(), QueryStatus::Success);
    }

    #[test]
    fn stale_request_is_rejected() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();
        let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        let id2 = r.begin_fetch_next(&mut seq, 2_000).unwrap();
        assert!(!r.complete_page_success(id1, vec!["stale".to_string()], true, true, 3_000));
        assert!(r.complete_page_success(id2, vec!["fresh".to_string()], false, true, 3_000));
    }

    #[test]
    fn max_pages_enforced_on_append() {
        let mut r = make_resource();
        r.set_max_pages(Some(2));
        let mut seq = RequestSequencer::new();

        let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id1, vec!["a".to_string()], true, true, 2_000);
        let id2 = r.begin_fetch_next(&mut seq, 3_000).unwrap();
        r.complete_page_success(id2, vec!["b".to_string()], true, true, 4_000);
        let id3 = r.begin_fetch_next(&mut seq, 5_000).unwrap();
        r.complete_page_success(id3, vec!["c".to_string()], false, true, 6_000);

        assert_eq!(r.page_count(), 2);
        assert_eq!(r.first_page(), Some(&vec!["b".to_string()]));
        assert_eq!(r.last_page(), Some(&vec!["c".to_string()]));
    }

    #[test]
    fn reset_clears_everything() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();
        let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id, vec!["page1".to_string()], true, true, 2_000);
        r.reset();
        assert!(r.pages().is_empty());
        assert_eq!(r.status(), QueryStatus::Idle);
    }

    #[test]
    fn invalidate_clears_last_updated() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();
        let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id, vec!["page1".to_string()], true, true, 2_000);
        assert!(r.last_updated_at_ms().is_some());
        r.invalidate();
        assert!(r.last_updated_at_ms().is_none());
        assert_eq!(r.page_count(), 1);
    }

    #[test]
    fn serde_roundtrip() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();
        let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id, vec!["a".to_string()], true, true, 2_000);
        let json = serde_json::to_string(&r).unwrap();
        let back: InfiniteQueryResource<Vec<String>> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.page_count(), 1);
        assert_eq!(back.status(), QueryStatus::Success);
        assert!(back.signal().is_none());
    }

    #[test]
    fn accept_current_request_returns_guard_for_active_request() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();
        let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();

        let guard = r.accept_current_request(id);
        assert!(guard.is_some());
        assert_eq!(r.active_request_id(), None);
    }

    #[test]
    fn accept_current_request_rejects_stale_request() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();
        let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        let _id2 = r.begin_fetch_next(&mut seq, 2_000).unwrap();

        let guard = r.accept_current_request(id1);
        assert!(guard.is_none());
    }

    #[test]
    fn complete_success_with_guard_appends_page() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();
        let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        let guard = r.accept_current_request(id).unwrap();

        r.complete_success_with_guard(guard, vec!["page1".to_string()], true, true, 2_000);
        assert_eq!(r.page_count(), 1);
        assert_eq!(r.status(), QueryStatus::Success);
        assert!(!r.is_fetching_next_page());
    }

    #[test]
    fn complete_failure_with_guard_preserves_pages() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();

        let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id1, vec!["page1".to_string()], true, true, 2_000);
        assert_eq!(r.page_count(), 1);

        let id2 = r.begin_fetch_next(&mut seq, 3_000).unwrap();
        let guard = r.accept_current_request(id2).unwrap();
        r.complete_failure_with_guard(guard, "network error".into());

        assert_eq!(r.status(), QueryStatus::Failure);
        assert_eq!(r.page_count(), 1);
        assert!(r.is_page_data_valid());
    }

    #[test]
    fn is_page_data_valid_idle_no_pages() {
        let r = make_resource();
        assert!(!r.is_page_data_valid());
    }

    #[test]
    fn is_page_data_valid_success_with_pages() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();
        let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id, vec!["page1".to_string()], true, true, 2_000);
        assert!(r.is_page_data_valid());
    }

    #[test]
    fn is_page_data_valid_failure_preserves_pages() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();

        let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id1, vec!["page1".to_string()], true, true, 2_000);

        let id2 = r.begin_fetch_next(&mut seq, 3_000).unwrap();
        r.complete_page_failure(id2, "network error".into());

        assert!(r.is_page_data_valid());
        assert_eq!(r.first_page(), Some(&vec!["page1".to_string()]));
    }

    #[test]
    fn is_page_data_valid_failure_no_pages() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();

        let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_failure(id, "network error".into());

        assert!(!r.is_page_data_valid());
    }

    #[test]
    fn max_pages_zero_treated_as_unbounded() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();

        let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id1, vec!["a".to_string()], true, true, 2_000);
        let id2 = r.begin_fetch_next(&mut seq, 3_000).unwrap();
        r.complete_page_success(id2, vec!["b".to_string()], true, true, 4_000);
        let id3 = r.begin_fetch_next(&mut seq, 5_000).unwrap();
        r.complete_page_success(id3, vec!["c".to_string()], true, true, 6_000);

        r.set_max_pages(Some(0));
        assert_eq!(r.max_pages(), None);
        assert_eq!(r.page_count(), 3);
    }

    #[test]
    fn set_max_pages_returns_evicted_pages() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();

        let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id1, vec!["a".to_string()], true, true, 2_000);
        let id2 = r.begin_fetch_next(&mut seq, 3_000).unwrap();
        r.complete_page_success(id2, vec!["b".to_string()], true, true, 4_000);
        let id3 = r.begin_fetch_next(&mut seq, 5_000).unwrap();
        r.complete_page_success(id3, vec!["c".to_string()], true, true, 6_000);

        let evicted = r.set_max_pages(Some(2));
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0].as_ref(), &vec!["a".to_string()]);
        assert_eq!(r.page_count(), 2);
    }

    #[test]
    fn append_page_returns_evicted_pages() {
        let mut r = make_resource();
        r.set_max_pages(Some(2));

        let evicted1 = r.append_page(vec!["a".to_string()]);
        assert!(evicted1.is_empty());

        let evicted2 = r.append_page(vec!["b".to_string()]);
        assert!(evicted2.is_empty());

        let evicted3 = r.append_page(vec!["c".to_string()]);
        assert_eq!(evicted3.len(), 1);
        assert_eq!(evicted3[0].as_ref(), &vec!["a".to_string()]);
        assert_eq!(r.page_count(), 2);
    }

    #[test]
    fn prepend_page_returns_evicted_pages() {
        let mut r = make_resource();
        r.set_max_pages(Some(2));

        r.prepend_page(vec!["a".to_string()]);
        r.prepend_page(vec!["b".to_string()]);

        let evicted = r.prepend_page(vec!["c".to_string()]);
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0].as_ref(), &vec!["a".to_string()]);
        assert_eq!(r.page_count(), 2);
        assert_eq!(r.first_page(), Some(&vec!["c".to_string()]));
    }

    #[test]
    fn stale_request_increments_ignored_results() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();

        let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        let id2 = r.begin_fetch_next(&mut seq, 2_000).unwrap();

        assert!(!r.complete_page_success(id1, vec!["stale".to_string()], true, true, 3_000));
        assert_eq!(r.ignored_results(), 1);

        assert!(r.complete_page_success(id2, vec!["fresh".to_string()], false, true, 3_000));
        assert_eq!(r.ignored_results(), 1);
    }

    #[test]
    fn stale_failure_increments_ignored_results() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();

        let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        let id2 = r.begin_fetch_next(&mut seq, 2_000).unwrap();

        assert!(!r.complete_page_failure(id1, "stale error".into()));
        assert_eq!(r.ignored_results(), 1);

        assert!(r.complete_page_failure(id2, "fresh error".into()));
        assert_eq!(r.ignored_results(), 1);
    }

    #[test]
    fn retry_count_accessors() {
        let mut r = make_resource();
        assert_eq!(r.retry_count(), 0);
        r.increment_retry();
        r.increment_retry();
        assert_eq!(r.retry_count(), 2);
        r.reset_retry_count();
        assert_eq!(r.retry_count(), 0);
    }

    #[test]
    fn reset_clears_diagnostics_but_preserves_max_pages() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();

        let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id, vec!["page1".to_string()], true, true, 2_000);
        r.set_max_pages(Some(10));
        r.increment_retry();
        r.increment_retry();

        r.reset();

        assert_eq!(r.max_pages(), Some(10));
        assert_eq!(r.retry_count(), 0);
        assert_eq!(r.ignored_results(), 0);
        assert_eq!(r.cancelled_count(), 0);
        assert!(r.has_next_page());
    }

    #[test]
    fn forward_only_defaults_has_next_true() {
        let r = make_resource();
        assert_eq!(r.direction(), FetchDirection::ForwardOnly);
        assert!(r.has_next_page());
        assert!(!r.has_previous_page());
    }

    #[test]
    fn bidirectional_defaults_both_false() {
        let r = make_bidirectional_resource();
        assert_eq!(r.direction(), FetchDirection::Bidirectional);
        assert!(!r.has_next_page());
        assert!(!r.has_previous_page());
    }

    #[test]
    fn bidirectional_begin_fetch_next_returns_none_without_opt_in() {
        let mut r = make_bidirectional_resource();
        let mut seq = RequestSequencer::new();
        let id = r.begin_fetch_next(&mut seq, 1_000);
        assert!(id.is_none());
    }

    #[test]
    fn bidirectional_fetch_works_after_opt_in() {
        let mut r = make_bidirectional_resource();
        let mut seq = RequestSequencer::new();
        r.set_has_next_page(true);
        let id = r.begin_fetch_next(&mut seq, 1_000);
        assert!(id.is_some());
    }

    #[test]
    fn reset_respects_direction() {
        let mut r = make_bidirectional_resource();
        let mut seq = RequestSequencer::new();
        r.set_has_next_page(true);
        let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id, vec!["page1".to_string()], true, true, 2_000);

        r.reset();
        assert!(!r.has_next_page());
        assert!(!r.has_previous_page());
        assert_eq!(r.direction(), FetchDirection::Bidirectional);
    }

    #[test]
    fn set_direction_changes_reset_behavior() {
        let mut r = make_resource();
        assert!(r.has_next_page());
        r.set_direction(FetchDirection::Bidirectional);
        r.reset();
        assert!(!r.has_next_page());
        assert!(!r.has_previous_page());
    }

    #[test]
    fn vec_deque_serde_roundtrip() {
        let mut r = make_resource();
        let mut seq = RequestSequencer::new();
        let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
        r.complete_page_success(id1, vec!["a".to_string()], true, true, 2_000);
        let id2 = r.begin_fetch_next(&mut seq, 3_000).unwrap();
        r.complete_page_success(id2, vec!["b".to_string()], true, true, 4_000);

        let json = serde_json::to_string(&r).unwrap();

        assert!(json.contains("\"pages\":["));
        assert!(!json.contains("VecDeque"));

        let back: InfiniteQueryResource<Vec<String>> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.page_count(), 2);
        assert_eq!(back.first_page(), Some(&vec!["a".to_string()]));
        assert_eq!(back.last_page(), Some(&vec!["b".to_string()]));
    }
}
