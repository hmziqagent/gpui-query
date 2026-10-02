use crate::core::*;

#[test]
fn cancel_during_loading_sets_failure() {
    let mut m: MutationResource<&'static str, i32> =
        MutationResource::new(RetryPolicy::no_retries());
    m.begin("vars");

    let signal = m.signal().unwrap().clone();
    assert!(!signal.is_cancelled());

    m.cancel(QueryError::cancelled("user aborted"));

    assert!(m.is_failure());
    assert_eq!(m.error().unwrap().kind(), QueryErrorKind::Cancelled);
    assert_eq!(m.error().unwrap().message(), "user aborted");
    assert!(
        signal.is_cancelled(),
        "external signal clone must see cancellation"
    );
    assert!(m.signal().is_none(), "signal cleared after cancel");
    assert_eq!(m.cancelled_count(), 1);
}

#[test]
fn cancel_on_idle_is_noop() {
    let mut m: MutationResource<&'static str, i32> =
        MutationResource::new(RetryPolicy::no_retries());
    m.cancel(QueryError::cancelled("x"));
    assert!(m.is_idle());
    assert_eq!(m.cancelled_count(), 0);
    assert!(m.error().is_none());
}

#[test]
fn cancel_on_success_is_noop() {
    let mut m: MutationResource<&'static str, i32> =
        MutationResource::new(RetryPolicy::no_retries());
    m.begin("vars");
    m.complete_success(10);
    m.cancel(QueryError::cancelled("x"));
    assert!(m.is_success());
    assert_eq!(m.data(), Some(&10));
    assert_eq!(m.cancelled_count(), 0);
}

#[test]
fn cancel_on_failure_is_noop() {
    let mut m: MutationResource<&'static str, i32> =
        MutationResource::new(RetryPolicy::no_retries());
    m.begin("vars");
    m.complete_failure(QueryError::response("fail"));
    let retry_count_before = m.retry_count();
    m.cancel(QueryError::cancelled("x"));
    assert!(m.is_failure());
    assert_eq!(m.retry_count(), retry_count_before);
    assert_eq!(m.cancelled_count(), 0);
}

#[test]
fn cancel_clears_data_from_previous_success() {
    let mut m: MutationResource<&'static str, i32> =
        MutationResource::new(RetryPolicy::no_retries());
    m.begin("vars");
    m.complete_success(42);
    m.begin("vars2");
    m.cancel(QueryError::cancelled("x"));
    assert!(m.is_failure());
    assert!(
        m.data().is_none(),
        "data from previous success must be cleared on cancel"
    );
}

#[test]
fn cancelled_count_increments_across_mutations() {
    let mut m: MutationResource<&'static str, i32> =
        MutationResource::new(RetryPolicy::no_retries());

    m.begin("first");
    m.cancel(QueryError::cancelled("abort 1"));
    assert_eq!(m.cancelled_count(), 1);

    m.begin("second");
    m.cancel(QueryError::cancelled("abort 2"));
    assert_eq!(m.cancelled_count(), 2);

    m.begin("third");
    m.cancel(QueryError::cancelled("abort 3"));
    assert_eq!(m.cancelled_count(), 3);
}

#[test]
fn cancel_stamps_last_updated_at_ms() {
    let mut m: MutationResource<&'static str, i32> =
        MutationResource::new(RetryPolicy::no_retries());
    m.begin("vars");
    assert!(m.last_updated_at_ms().is_none());
    m.cancel(QueryError::cancelled("user aborted"));
    assert!(
        m.last_updated_at_ms().is_some(),
        "cancel is a terminal completion and must refresh GC recency"
    );
}
