use crate::core::*;

#[test]
fn retry_from_failure_goes_to_loading() {
    let mut m: MutationResource<&'static str, i32> = MutationResource::new(RetryPolicy::new(2));

    m.begin("vars");
    m.complete_failure(QueryError::transport("timeout"));

    assert!(m.is_failure());
    assert!(m.should_retry(), "retry_count=1 < max_retries=2");
    assert_eq!(m.retry_count(), 1);

    let retried = m.retry();
    assert!(retried);
    assert!(m.is_loading());
    assert!(m.error().is_none(), "retry clears error");
    assert!(m.signal().is_some(), "retry creates fresh signal");
    assert!(!m.signal().unwrap().is_cancelled());
    assert_eq!(m.variables(), Some(&"vars"), "variables preserved");
}

#[test]
fn retry_respects_max_retries() {
    let mut m: MutationResource<&'static str, i32> = MutationResource::new(RetryPolicy::new(2));

    m.begin("vars");

    m.complete_failure(QueryError::response("fail 1"));
    assert_eq!(m.retry_count(), 1);
    assert!(m.should_retry(), "1 < 2");
    assert!(m.retry());

    m.complete_failure(QueryError::response("fail 2"));
    assert_eq!(m.retry_count(), 2);
    assert!(!m.should_retry(), "2 is not < 2");
    assert!(!m.retry(), "retry should fail when max reached");

    assert!(m.is_failure());
    assert_eq!(m.error().unwrap().message(), "fail 2");
}

#[test]
fn retry_only_from_failure_status() {
    let mut m: MutationResource<&'static str, i32> = MutationResource::new(RetryPolicy::new(3));

    assert!(!m.retry());

    m.begin("vars");
    assert!(!m.retry());

    m.complete_failure(QueryError::response("fail"));
    assert!(m.retry());

    assert!(m.is_loading());
    assert!(!m.retry());
}

#[test]
fn retry_creates_fresh_signal() {
    let mut m: MutationResource<&'static str, i32> = MutationResource::new(RetryPolicy::new(2));
    m.begin("vars");
    let old_signal = m.signal().unwrap().clone();

    m.complete_failure(QueryError::response("fail"));

    assert!(m.signal().is_none());

    let retried = m.retry();
    assert!(retried);

    let new_signal = m.signal().unwrap();
    assert!(
        !new_signal.is_cancelled(),
        "fresh signal after retry must not be cancelled"
    );
    assert_ne!(old_signal, *new_signal, "signals must be different objects");
}

#[test]
fn prepare_retry_stays_in_loading_with_fresh_signal() {
    let mut m: MutationResource<&'static str, i32> = MutationResource::new(RetryPolicy::new(3));
    m.begin("vars");

    let old_signal = m.signal().unwrap().clone();
    assert!(!old_signal.is_cancelled());

    m.prepare_retry();

    assert!(m.is_loading(), "prepare_retry must not leave Loading");
    assert!(m.error().is_none());
    assert!(
        old_signal.is_cancelled(),
        "old signal must be cancelled by prepare_retry"
    );

    let new_signal = m.signal().unwrap();
    assert!(!new_signal.is_cancelled(), "new signal must be fresh");
}

#[test]
fn prepare_retry_noop_when_not_loading() {
    let mut m: MutationResource<&'static str, i32> = MutationResource::new(RetryPolicy::new(3));

    m.prepare_retry();
    assert!(m.is_idle());

    m.begin("vars");
    m.complete_success(42);
    m.prepare_retry();
    assert!(m.is_success());
    assert_eq!(m.data(), Some(&42));

    m.begin("vars");
    m.complete_failure(QueryError::response("fail"));
    m.prepare_retry();
    assert!(m.is_failure());
}

#[test]
fn increment_retry_bumps_counter_without_state_change() {
    let mut m: MutationResource<&'static str, i32> = MutationResource::new(RetryPolicy::new(5));
    m.begin("vars");

    assert_eq!(m.retry_count(), 0);
    assert!(m.is_loading());

    m.increment_retry();
    assert_eq!(m.retry_count(), 1);
    assert!(
        m.is_loading(),
        "must still be Loading after increment_retry"
    );

    m.increment_retry();
    m.increment_retry();
    assert_eq!(m.retry_count(), 3);
}

#[test]
fn reset_retry_count_zeroes_counter() {
    let mut m: MutationResource<&'static str, i32> = MutationResource::new(RetryPolicy::new(5));
    m.begin("vars");
    m.increment_retry();
    m.increment_retry();
    assert_eq!(m.retry_count(), 2);

    m.reset_retry_count();
    assert_eq!(m.retry_count(), 0);
}

#[test]
fn retry_count_increments_saturating_on_complete_failure() {
    let mut m: MutationResource<&'static str, i32> = MutationResource::new(RetryPolicy::new(100));
    m.begin("vars");

    for i in 1..=10u32 {
        m.complete_failure(QueryError::response("fail"));
        assert_eq!(
            m.retry_count(),
            i,
            "retry_count must increment on each failure"
        );
        if i < 10 {
            let retried = m.retry();
            assert!(retried, "retry must succeed while retries remain");
        }
    }
    assert_eq!(m.retry_count(), 10);
}

#[test]
fn retry_allowed_after_cancel_sets_failure() {
    let mut m: MutationResource<&'static str, i32> = MutationResource::new(RetryPolicy::new(3));
    m.begin("vars");
    m.cancel(QueryError::cancelled("abort"));

    assert_eq!(m.retry_count(), 0, "cancel does not increment retry_count");
    assert!(m.should_retry(), "retry_count=0 < max_retries=3");
    assert!(m.retry(), "retry from cancelled Failure should work");
    assert!(m.is_loading());
}
