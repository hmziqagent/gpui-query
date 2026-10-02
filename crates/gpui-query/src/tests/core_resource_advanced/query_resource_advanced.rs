use crate::core::*;
use crate::tests::test_support::*;
use std::num::NonZero;

#[test]
fn failure_to_success_recovery_cycle() {
    let mut r: QueryResource<&str> =
        QueryResource::new("test", CachePolicy::NoCache, RequestPolicy::LatestWins);
    let mut s = test_sequencer();

    let rid1 = match r.begin_request(&mut s, 100, QueryFetchMode::Normal) {
        QueryBeginResult::Started { request_id, .. } => request_id,
        _ => panic!("expected Started"),
    };
    r.complete_current_success(rid1, "v1", 200);

    let rid2 = match r.begin_request(&mut s, 1_500, QueryFetchMode::Normal) {
        QueryBeginResult::Started { request_id, .. } => request_id,
        _ => panic!("expected Started"),
    };
    r.complete_current_failure(rid2, QueryError::transport("timeout"), 1_600);
    assert_eq!(r.status(), QueryStatus::Failure);
    assert_eq!(r.data(), Some(&"v1"), "failure retains prior data");

    let rid3 = match r.begin_request(&mut s, 2_000, QueryFetchMode::Normal) {
        QueryBeginResult::Started { request_id, .. } => request_id,
        _ => panic!("expected Started"),
    };
    r.complete_current_success(rid3, "v2", 2_100);
    assert_eq!(r.status(), QueryStatus::Success);
    assert_eq!(r.data(), Some(&"v2"));
    assert!(r.error().is_none());
    assert_eq!(r.previous_data(), Some(&"v1"));
}

#[test]
fn failure_with_data_then_success_updates_data() {
    let mut r: QueryResource<&str> =
        QueryResource::new("test", CachePolicy::NoCache, RequestPolicy::LatestWins);
    let mut s = test_sequencer();

    let rid = match r.begin_request(&mut s, 100, QueryFetchMode::Normal) {
        QueryBeginResult::Started { request_id, .. } => request_id,
        _ => panic!("expected Started"),
    };
    r.complete_current_failure_with_data(
        rid,
        "fallback",
        QueryError::response("partial fail"),
        200,
    );
    assert_eq!(r.status(), QueryStatus::Failure);
    assert_eq!(r.data(), Some(&"fallback"));

    let rid2 = match r.begin_request(&mut s, 300, QueryFetchMode::Normal) {
        QueryBeginResult::Started { request_id, .. } => request_id,
        _ => panic!("expected Started"),
    };
    r.complete_current_success(rid2, "fresh", 400);
    assert_eq!(r.status(), QueryStatus::Success);
    assert_eq!(r.data(), Some(&"fresh"));
    assert!(r.error().is_none());
}

#[test]
fn signal_mut_returns_signal_when_active() {
    let mut r = test_resource();
    let mut s = test_sequencer();
    let _ = r.begin_request(&mut s, 100, QueryFetchMode::Normal);

    let sig = r.signal_mut();
    assert!(sig.is_some());
    assert!(!sig.unwrap().is_cancelled());
}

#[test]
fn signal_mut_returns_none_when_idle() {
    let mut r = test_resource();
    assert!(r.signal_mut().is_none());
}

#[test]
fn set_retry_policy_updates_policy() {
    let mut r = test_resource();
    assert_eq!(r.retry_policy().max_retries, 0);

    let new_policy = RetryPolicy::new(5)
        .with_delay(200)
        .with_exponential_backoff();
    r.set_retry_policy(new_policy.clone());
    assert_eq!(r.retry_policy(), &new_policy);
    assert_eq!(r.retry_policy().max_retries, 5);
}

#[test]
fn retry_policy_preserved_across_reset() {
    let mut r = test_resource();
    r.set_retry_policy(RetryPolicy::new(10));
    r.increment_retry();
    r.reset();
    assert_eq!(
        r.retry_policy().max_retries,
        10,
        "policy preserved after reset"
    );
    assert_eq!(r.retry_count(), 0, "count cleared after reset");
}

#[test]
fn serde_roundtrip_with_data_and_error_state() {
    let mut r: QueryResource<String, QueryError> = QueryResource::new(
        "serde-test",
        CachePolicy::NoCache,
        RequestPolicy::LatestWins,
    );
    let mut s = test_sequencer();
    let rid = begin_request_id(&mut r, &mut s, 100, QueryFetchMode::Normal);
    r.complete_current_success(rid, "hello".to_string(), 200);

    let json = serde_json::to_string(&r).unwrap();
    let back: QueryResource<String, QueryError> = serde_json::from_str(&json).unwrap();

    assert_eq!(back.status(), QueryStatus::Success);
    assert_eq!(back.data(), Some(&"hello".to_string()));
    assert_eq!(back.key().first_segment(), "serde-test");
    assert_eq!(back.cache_policy(), CachePolicy::NoCache);
    assert_eq!(back.request_policy(), RequestPolicy::LatestWins);
    assert!(back.signal().is_none(), "signal is #[serde(skip)]");

    let rid2 = begin_request_id(&mut r, &mut s, 300, QueryFetchMode::Normal);
    r.complete_current_failure(rid2, QueryError::transport("fail"), 400);
    let json2 = serde_json::to_string(&r).unwrap();
    let back2: QueryResource<String, QueryError> = serde_json::from_str(&json2).unwrap();
    assert_eq!(back2.status(), QueryStatus::Failure);
    assert!(back2.error().is_some());
    assert!(back2.signal().is_none());
}

#[test]
fn mark_ignored_result_increments_counter() {
    let mut r = test_resource();
    assert_eq!(r.ignored_results(), 0);

    r.mark_ignored_result();
    assert_eq!(r.ignored_results(), 1);

    r.mark_ignored_result();
    r.mark_ignored_result();
    assert_eq!(r.ignored_results(), 3);
}

#[test]
fn set_request_policy_changes_policy() {
    let mut r = test_resource();
    assert_eq!(r.request_policy(), RequestPolicy::LatestWins);

    r.set_request_policy(RequestPolicy::IgnoreWhileLoading);
    assert_eq!(r.request_policy(), RequestPolicy::IgnoreWhileLoading);
}

#[test]
fn set_request_policy_preserved_across_reset() {
    let mut r = test_resource();
    r.set_request_policy(RequestPolicy::IgnoreWhileLoading);
    r.reset();
    assert_eq!(r.request_policy(), RequestPolicy::IgnoreWhileLoading);
}

#[test]
fn ttl_resource_should_not_clear_data_on_complete() {
    let r = test_resource();
    assert!(!r.should_clear_data_on_complete());
}

#[test]
fn cancelled_to_loading_to_success_recovery() {
    let mut r = test_resource();
    let mut s = test_sequencer();

    let _ = r.begin_request(&mut s, 100, QueryFetchMode::Normal);
    r.cancel(QueryError::cancelled("abort"));
    assert_eq!(r.status(), QueryStatus::Cancelled);

    let rid2 = match r.begin_request(&mut s, 200, QueryFetchMode::Normal) {
        QueryBeginResult::Started { request_id, .. } => request_id,
        _ => panic!("expected Started"),
    };
    assert_eq!(r.status(), QueryStatus::LoadingEmpty);
    assert!(r.error().is_none(), "begin clears error");

    r.complete_current_success(rid2, "recovered", 300);
    assert_eq!(r.status(), QueryStatus::Success);
    assert_eq!(r.data(), Some(&"recovered"));
}

#[test]
fn begin_request_with_id_respects_ignore_while_loading() {
    let mut r: QueryResource<&str> = QueryResource::new(
        "test",
        CachePolicy::NoCache,
        RequestPolicy::IgnoreWhileLoading,
    );
    let custom_id = RequestId::scoped(NonZero::new(10).unwrap(), 1);
    let _ = r.begin_request_with_id(Some(custom_id), 100, QueryFetchMode::Normal);
    assert_eq!(r.active_request_id(), Some(custom_id));

    let result = r.begin_request_with_id(
        Some(RequestId::scoped(NonZero::new(10).unwrap(), 2)),
        200,
        QueryFetchMode::Normal,
    );
    match result {
        QueryBeginResult::IgnoredWhileLoading { active_request_id } => {
            assert_eq!(active_request_id, custom_id);
        }
        _ => panic!("expected IgnoredWhileLoading, got {:?}", result),
    }
    assert_eq!(r.active_request_id(), Some(custom_id));
}
