use crate::core::{
    CachePolicy, QueryBeginResult, QueryFetchMode, QueryResource, QueryStatus, RequestId,
    RequestPolicy, RequestSequencer,
};
use crate::tests::test_support::{
    TEST_NOW_MS, assert_status, begin_request_id, test_resource_with_policies, test_sequencer,
};
use std::num::NonZero;

#[test]
fn guard_holds_the_correct_request_id() {
    let mut resource: QueryResource<&str> =
        test_resource_with_policies("key", CachePolicy::NoCache, RequestPolicy::LatestWins);
    let mut seq = test_sequencer();

    let rid = begin_request_id(&mut resource, &mut seq, TEST_NOW_MS, QueryFetchMode::Normal);

    let guard = resource.accept_current_request(rid).unwrap();
    assert_eq!(guard.request_id(), rid);
}

#[test]
fn guard_into_request_id_consumes_guard() {
    let mut resource: QueryResource<&str> =
        test_resource_with_policies("key", CachePolicy::NoCache, RequestPolicy::LatestWins);
    let mut seq = test_sequencer();

    let rid = begin_request_id(&mut resource, &mut seq, TEST_NOW_MS, QueryFetchMode::Normal);

    let guard = resource.accept_current_request(rid).unwrap();
    let extracted = guard.into_request_id();
    assert_eq!(extracted, rid);
}

#[test]
fn accept_current_request_rejects_stale_request_id() {
    let mut resource: QueryResource<&str> =
        test_resource_with_policies("key", CachePolicy::NoCache, RequestPolicy::LatestWins);
    let mut seq = test_sequencer();

    let old_id = begin_request_id(&mut resource, &mut seq, TEST_NOW_MS, QueryFetchMode::Normal);

    let result = resource.begin_request(&mut seq, TEST_NOW_MS, QueryFetchMode::Normal);
    let _new_id = match result {
        QueryBeginResult::Started {
            request_id,
            replaced_request_id,
            ..
        } => {
            assert_eq!(replaced_request_id, Some(old_id));
            request_id
        }
        other => panic!("expected Started, got {:?}", other),
    };

    let result = resource.accept_current_request(old_id);
    assert!(
        result.is_none(),
        "stale request id should be rejected, but got a guard"
    );
    assert_eq!(resource.ignored_results(), 1);
}

#[test]
fn complete_success_consumes_guard_and_sets_data() {
    let mut resource: QueryResource<&str> =
        test_resource_with_policies("key", CachePolicy::NoCache, RequestPolicy::LatestWins);
    let mut seq = test_sequencer();

    let rid = begin_request_id(&mut resource, &mut seq, TEST_NOW_MS, QueryFetchMode::Normal);

    let guard = resource.accept_current_request(rid).unwrap();
    resource.complete_success(guard, "result", TEST_NOW_MS);

    assert_eq!(resource.data(), Some(&"result"));
    assert_status(&resource, QueryStatus::Success);
    assert_eq!(resource.active_request_id(), None);
}

#[test]
fn complete_failure_consumes_guard_and_sets_error() {
    let mut resource: QueryResource<&str> =
        test_resource_with_policies("key", CachePolicy::NoCache, RequestPolicy::LatestWins);
    let mut seq = test_sequencer();

    let rid = begin_request_id(&mut resource, &mut seq, TEST_NOW_MS, QueryFetchMode::Normal);

    let guard = resource.accept_current_request(rid).unwrap();
    resource.complete_failure(guard, "network error", TEST_NOW_MS);

    assert!(resource.data().is_none());
    assert_status(&resource, QueryStatus::Failure);
    assert!(resource.error().is_some());
}

#[test]
fn complete_convenience_method_rejects_stale_id() {
    let mut resource: QueryResource<&str> =
        test_resource_with_policies("key", CachePolicy::NoCache, RequestPolicy::LatestWins);
    let mut seq = test_sequencer();

    let old_id = begin_request_id(&mut resource, &mut seq, TEST_NOW_MS, QueryFetchMode::Normal);

    let _ = resource.begin_request(&mut seq, TEST_NOW_MS, QueryFetchMode::Normal);

    let completed = resource.complete_current_success(old_id, "stale data", TEST_NOW_MS);
    assert!(!completed, "stale request should not be completed");
    assert!(
        resource.data().is_none(),
        "data should remain unset after stale completion attempt"
    );
}

#[test]
fn begin_request_with_id_uses_provided_id() {
    let mut resource: QueryResource<&str> =
        test_resource_with_policies("key", CachePolicy::NoCache, RequestPolicy::LatestWins);
    let custom_id = RequestId::scoped(NonZero::new(99).unwrap(), 7);

    let result =
        resource.begin_request_with_id(Some(custom_id), TEST_NOW_MS, QueryFetchMode::Normal);
    let rid = match result {
        QueryBeginResult::Started { request_id, .. } => request_id,
        other => panic!("expected Started, got {:?}", other),
    };
    assert_eq!(rid, custom_id);
}

#[test]
fn begin_request_with_id_none_mints_in_reserved_fallback_scope() {
    let mut resource: QueryResource<&str> =
        test_resource_with_policies("key", CachePolicy::NoCache, RequestPolicy::LatestWins);

    let result = resource.begin_request_with_id(None, TEST_NOW_MS, QueryFetchMode::Normal);
    let rid = match result {
        QueryBeginResult::Started { request_id, .. } => request_id,
        other => panic!("expected Started, got {:?}", other),
    };
    assert_eq!(rid.scope_id(), RequestSequencer::RESERVED_FALLBACK_SCOPE);
    assert_eq!(rid.value(), 1);
}

#[test]
fn fallback_ids_stay_in_reserved_scope_across_mints() {
    let mut resource: QueryResource<&str> =
        test_resource_with_policies("key", CachePolicy::NoCache, RequestPolicy::LatestWins);

    let first = match resource.begin_request_with_id(None, TEST_NOW_MS, QueryFetchMode::Normal) {
        QueryBeginResult::Started { request_id, .. } => request_id,
        other => panic!("expected Started, got {:?}", other),
    };
    let _ = resource.accept_current_request(first).unwrap();
    let second = match resource.begin_request_with_id(None, TEST_NOW_MS, QueryFetchMode::Normal) {
        QueryBeginResult::Started { request_id, .. } => request_id,
        other => panic!("expected Started, got {:?}", other),
    };

    assert_eq!(
        second.scope_id(),
        RequestSequencer::RESERVED_FALLBACK_SCOPE,
        "successive fallback mints must stay in the reserved scope"
    );
    assert_eq!(second.value(), 2);
    assert_ne!(first, second);
}

#[test]
fn begin_request_with_id_swr_ignore_while_loading_keeps_active_request() {
    let mut r: QueryResource<&str> = QueryResource::new(
        "swr-ignore",
        CachePolicy::StaleWhileRevalidate {
            ttl_ms: 500,
            stale_ms: 1_000,
        },
        RequestPolicy::IgnoreWhileLoading,
    );
    let mut seq = test_sequencer();

    r.apply_success("cached", 100);

    let _ = r.begin_request(&mut seq, 1_500, QueryFetchMode::Force);
    assert!(r.is_loading());

    let result = r.begin_request_with_id(
        Some(RequestId::scoped(NonZero::new(99).unwrap(), 1)),
        1_500,
        QueryFetchMode::Normal,
    );

    match result {
        QueryBeginResult::StaleCacheHit {
            request_id,
            replaced_request_id,
            ..
        } => {
            assert!(
                replaced_request_id.is_none(),
                "no replacement under IgnoreWhileLoading"
            );
            assert_ne!(
                request_id,
                RequestId::scoped(NonZero::new(99).unwrap(), 1),
                "should use existing active request id"
            );
        }
        other => panic!("expected StaleCacheHit, got {:?}", other),
    }
}
