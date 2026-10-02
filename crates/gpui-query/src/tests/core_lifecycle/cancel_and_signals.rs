use crate::core::*;
use crate::tests::core_lifecycle::transitions::*;

#[test]
fn cancel_from_loading_empty() {
    let mut r = resource();
    let mut s = seq();

    let (rid, _) = begin(&mut r, &mut s, 100);

    assert!(r.cancel(QueryError::cancelled("user abort")));

    assert_eq!(r.status(), QueryStatus::Cancelled);
    assert!(r.active_request_id().is_none());
    assert_eq!(r.data(), None);
    assert_eq!(err_str(&r), Some("cancelled: user abort".to_string()));
    assert_eq!(r.cancelled_count(), 1);
    assert!(r.accept_current_request(rid).is_none());
}

#[test]
fn cancel_from_loading_with_data_saves_previous_data() {
    let mut r = resource();
    let mut s = seq();

    let (rid, _) = begin(&mut r, &mut s, 100);
    assert!(r.complete_current_success(rid, "cached", 200));

    let (_rid2, _) = begin(&mut r, &mut s, 1_500);
    assert!(r.cancel(QueryError::cancelled("aborted")));

    assert_eq!(r.status(), QueryStatus::Cancelled);
    assert_eq!(r.data(), None, "cancel clears data");
    assert_eq!(
        r.previous_data(),
        Some(&"cached"),
        "cancel saves prior data to previous_data for rollback"
    );
    assert_eq!(r.last_updated_at_ms(), Some(200), "timestamp preserved");
}

#[test]
fn rollback_to_previous_restores_data_after_cancel() {
    let mut r = resource();
    let mut s = seq();

    let (rid, _) = begin(&mut r, &mut s, 100);
    assert!(r.complete_current_success(rid, "cached", 200));

    let (_rid2, _) = begin(&mut r, &mut s, 1_500);
    assert!(r.cancel(QueryError::cancelled("aborted")));

    assert!(r.rollback_to_previous());

    assert_eq!(r.status(), QueryStatus::Success);
    assert_eq!(r.data(), Some(&"cached"));
    assert_eq!(r.previous_data(), None);
}

#[test]
fn cancel_without_active_request_returns_false() {
    let mut r = resource();

    assert!(!r.cancel(QueryError::cancelled("nope")));
    assert_eq!(r.status(), QueryStatus::Idle);
    assert_eq!(r.cancelled_count(), 0);
}

#[test]
fn cancel_after_completion_is_noop() {
    let mut r = resource();
    let mut s = seq();

    let (rid, _) = begin(&mut r, &mut s, 100);
    assert!(r.complete_current_success(rid, "done", 200));

    assert!(!r.cancel(QueryError::cancelled("late")));
    assert_eq!(r.status(), QueryStatus::Success);
    assert_eq!(r.data(), Some(&"done"));
    assert_eq!(r.cancelled_count(), 0);
}

#[test]
fn begin_request_creates_fresh_signal() {
    let mut r = resource();
    let mut s = seq();

    assert!(r.signal().is_none(), "no signal before first request");

    let _ = r.begin_request(&mut s, 100, QueryFetchMode::Normal);

    let sig = r.signal().expect("signal must exist after begin_request");
    assert!(!sig.is_cancelled(), "fresh signal must not be cancelled");
}

#[test]
fn cancel_propagates_to_signal() {
    let mut r = resource();
    let mut s = seq();

    let _ = r.begin_request(&mut s, 100, QueryFetchMode::Normal);
    let clone = r.signal().unwrap().clone();

    assert!(r.cancel(QueryError::cancelled("aborted")));

    assert!(clone.is_cancelled(), "cloned signal must see cancellation");
    assert!(r.signal().unwrap().is_cancelled());
}

#[test]
fn new_request_cancels_old_signal() {
    let mut r = resource();
    let mut s = seq();

    let _ = r.begin_request(&mut s, 100, QueryFetchMode::Normal);
    let old_signal = r.signal().unwrap().clone();

    let _ = r.begin_request(&mut s, 200, QueryFetchMode::Normal);

    assert!(
        old_signal.is_cancelled(),
        "old signal must be cancelled on replacement"
    );
    let new_signal = r.signal().unwrap();
    assert!(!new_signal.is_cancelled(), "new signal must be fresh");
    assert_ne!(old_signal, *new_signal, "signals must be distinct objects");
}

#[test]
fn completion_does_not_cancel_signal() {
    let mut r = resource();
    let mut s = seq();

    let result = r.begin_request(&mut s, 100, QueryFetchMode::Normal);
    let rid = match result {
        QueryBeginResult::Started { request_id, .. } => request_id,
        _ => panic!("expected Started"),
    };

    assert!(r.complete_current_success(rid, "data", 200));

    let sig = r.signal().expect("signal persists after completion");
    assert!(!sig.is_cancelled(), "completion should not cancel signal");
}
