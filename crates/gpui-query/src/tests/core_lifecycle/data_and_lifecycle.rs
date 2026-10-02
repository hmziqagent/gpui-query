use crate::core::*;
use crate::tests::core_lifecycle::transitions::*;
use crate::tests::test_support::nocache_resource;

#[test]
fn invalidate_clears_timestamp_but_retains_data_and_active_request() {
    let mut r = resource();
    let mut s = seq();

    let (rid, _) = begin(&mut r, &mut s, 100);
    assert!(r.complete_current_success(rid, "data", 200));

    let (_rid2, _) = begin(&mut r, &mut s, 1_500);

    r.invalidate();

    assert_eq!(r.data(), Some(&"data"));
    assert!(
        r.active_request_id().is_some(),
        "invalidate does not cancel active request"
    );
    assert_eq!(r.last_updated_at_ms(), None, "invalidate clears timestamp");
}

#[test]
fn cancelled_count_increments_on_each_replacement() {
    let mut r = resource();
    let mut s = seq();

    let _ = begin(&mut r, &mut s, 100);
    let _ = begin(&mut r, &mut s, 200);
    let _ = begin(&mut r, &mut s, 300);

    assert_eq!(r.cancelled_count(), 2, "two requests were replaced");
}

#[test]
fn cancelled_count_includes_explicit_cancel() {
    let mut r = resource();
    let mut s = seq();

    let _ = begin(&mut r, &mut s, 100);
    assert!(r.cancel(QueryError::cancelled("abort")));

    assert_eq!(r.cancelled_count(), 1);
}

#[test]
fn accept_then_complete_success_via_guard() {
    let mut r = resource();
    let mut s = seq();

    let (rid, _) = begin(&mut r, &mut s, 100);

    let guard = r
        .accept_current_request(rid)
        .expect("should accept current request");
    assert!(
        r.active_request_id().is_none(),
        "accept clears active_request_id"
    );

    r.complete_success(guard, "data", 200);

    assert_eq!(r.status(), QueryStatus::Success);
    assert_eq!(r.data(), Some(&"data"));
    assert_eq!(r.last_updated_at_ms(), Some(200));
}

#[test]
fn accept_then_complete_failure_via_guard() {
    let mut r = resource();
    let mut s = seq();

    let (rid, _) = begin(&mut r, &mut s, 100);
    let guard = r.accept_current_request(rid).expect("should accept");

    r.complete_failure(guard, QueryError::transport("net error"), 200);

    assert_eq!(r.status(), QueryStatus::Failure);
    assert_eq!(err_str(&r), Some("transport error: net error".to_string()));
}

#[test]
fn is_current_request_matches_active() {
    let mut r = resource();
    let mut s = seq();

    let (rid1, _) = begin(&mut r, &mut s, 100);
    assert!(r.is_current_request(rid1));

    let (rid2, _) = begin(&mut r, &mut s, 200);
    assert!(!r.is_current_request(rid1), "rid1 is stale");
    assert!(r.is_current_request(rid2), "rid2 is current");
}

#[test]
fn complete_success_optional_none_yields_idle_some_yields_success() {
    {
        let mut r = nocache_resource("optional-none");
        let mut s = seq();
        let (rid, _) = begin(&mut r, &mut s, 100);
        let guard = r.accept_current_request(rid).unwrap();
        r.complete_success_optional(guard, None, 200);
        assert_eq!(r.status(), QueryStatus::Idle, "None data => Idle");
        assert!(r.data().is_none());
        assert!(r.error().is_none());
    }

    {
        let mut r = nocache_resource("optional-some");
        let mut s = seq();
        let (rid, _) = begin(&mut r, &mut s, 100);
        let guard = r.accept_current_request(rid).unwrap();
        r.complete_success_optional(guard, Some("data"), 200);
        assert_eq!(r.status(), QueryStatus::Success);
        assert_eq!(r.data(), Some(&"data"));
    }
}

#[test]
fn is_data_stale_by_status() {
    let mut r = nocache_resource("stale-heuristic");
    assert!(!r.is_data_stale(), "no data => not stale");

    let mut s = seq();
    let (rid, _) = begin(&mut r, &mut s, 100);
    r.complete_current_success(rid, "data", 200);
    assert!(!r.is_data_stale(), "Success with data => not stale");

    let _ = begin(&mut r, &mut s, 300);
    assert_eq!(r.status(), QueryStatus::LoadingWithData);
    assert!(r.is_data_stale(), "LoadingWithData with data => stale");

    let (rid2, _) = begin(&mut r, &mut s, 400);
    r.complete_current_failure_with_data(rid2, "fallback", QueryError::response("err"), 500);
    assert_eq!(r.status(), QueryStatus::Failure);
    assert!(r.is_data_stale(), "Failure with data => stale");
}

#[test]
fn full_lifecycle_round_trip() {
    let mut r = resource();
    let mut s = seq();

    assert_eq!(r.status(), QueryStatus::Idle);
    let (rid1, status1) = begin(&mut r, &mut s, 100);
    assert_eq!(status1, QueryStatus::LoadingEmpty);

    assert!(r.complete_current_success(rid1, "v1", 200));
    assert_eq!(r.status(), QueryStatus::Success);

    let (rid2, status2) = begin(&mut r, &mut s, 1_500);
    assert_eq!(status2, QueryStatus::LoadingWithData);
    assert_eq!(r.data(), Some(&"v1"));

    assert!(r.complete_current_success(rid2, "v2", 1_600));
    assert_eq!(r.status(), QueryStatus::Success);
    assert_eq!(r.data(), Some(&"v2"));
    assert_eq!(r.previous_data(), Some(&"v1"));

    let (_rid3, _) = begin(&mut r, &mut s, 3_000);
    assert_eq!(r.status(), QueryStatus::LoadingWithData);
    assert!(r.cancel(QueryError::cancelled("manual")));
    assert_eq!(r.status(), QueryStatus::Cancelled);
    assert_eq!(r.data(), None);
    assert_eq!(r.previous_data(), Some(&"v2"));

    assert!(r.rollback_to_previous());
    assert_eq!(r.status(), QueryStatus::Success);
    assert_eq!(r.data(), Some(&"v2"));

    r.reset();
    assert_eq!(r.status(), QueryStatus::Idle);
    assert_eq!(r.data(), None);
    assert_eq!(r.cancelled_count(), 0);
}

#[test]
fn data_epoch_counts_writes_not_value_changes() {
    let mut r = resource();
    assert_eq!(r.data_epoch(), 0);

    r.set_data("v");
    assert_eq!(r.data_epoch(), 1);
    r.set_data("v");
    assert_eq!(
        r.data_epoch(),
        2,
        "equal-value overwrite still counts as a write"
    );

    r.clear_data();
    assert_eq!(r.data_epoch(), 3);

    assert!(r.rollback_to_previous());
    assert_eq!(r.data_epoch(), 4);
}

#[test]
fn data_epoch_bumps_on_success_and_failure_with_data_paths() {
    let mut r = nocache_resource("epoch-completion");
    let mut s = seq();

    let (rid, _) = begin(&mut r, &mut s, 100);
    r.complete_current_optional_success(rid, Some("a"), 200);
    assert_eq!(r.data_epoch(), 1);

    let (rid2, _) = begin(&mut r, &mut s, 300);
    r.complete_current_failure_with_data(rid2, "b", QueryError::response("x"), 400);
    assert_eq!(r.data_epoch(), 2);
    assert_eq!(r.data(), Some(&"b"));
}

#[test]
fn data_epoch_unchanged_by_failure_without_data() {
    let mut r = nocache_resource("epoch-failure");
    let mut s = seq();

    let (rid, _) = begin(&mut r, &mut s, 100);
    r.complete_current_failure(rid, QueryError::response("x"), 200);

    assert_eq!(r.data_epoch(), 0);
    assert_eq!(r.status(), QueryStatus::Failure);
}

#[test]
fn data_epoch_bumps_when_cancel_or_reset_moves_data_out() {
    let mut r = nocache_resource("epoch-cancel-reset");
    let mut s = seq();

    let (rid, _) = begin(&mut r, &mut s, 100);
    assert!(r.complete_current_success(rid, "v", 200));
    assert_eq!(r.data_epoch(), 1);

    let (_rid2, _) = begin(&mut r, &mut s, 300);
    assert!(r.cancel(QueryError::response("c")));
    assert_eq!(r.data(), None);
    assert_eq!(r.data_epoch(), 2);

    assert!(r.rollback_to_previous());
    assert_eq!(r.data(), Some(&"v"));
    assert_eq!(r.data_epoch(), 3);

    r.reset();
    assert_eq!(r.data(), None);
    assert_eq!(r.data_epoch(), 4);

    r.reset();
    assert_eq!(r.data_epoch(), 4, "reset without data is not a data write");
}

#[test]
fn equality_ignores_the_data_epoch() {
    let mut a = resource();
    let mut s = seq();

    let (rid, _) = begin(&mut a, &mut s, 100);
    assert!(a.complete_current_success(rid, "v", 200));
    let mut b = a.clone();
    assert_eq!(a, b);

    a.set_data("v");
    assert!(a.rollback_to_previous());
    assert_eq!(
        a, b,
        "a same-value write + rollback leaves equal observable state despite \
         different data epochs"
    );

    b.set_data("w");
    assert_ne!(a, b);
}
