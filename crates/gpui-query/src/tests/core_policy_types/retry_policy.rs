use crate::core::*;
use crate::tests::test_support::assert_serde_roundtrip;

#[test]
fn retry_policy_default_is_3_with_exponential() {
    let policy = RetryPolicy::default();
    assert_eq!(policy.max_retries, 3);
    assert!(policy.exponential_backoff);
    assert_eq!(policy.retry_delay_ms, 1000);
    assert_eq!(policy.max_retry_delay_ms, 30_000);
}

#[test]
fn retry_policy_no_retries_has_zero_delay() {
    let policy = RetryPolicy::no_retries();
    assert_eq!(policy.max_retries, 0);
    assert_eq!(policy.retry_delay_ms, 0);
    assert!(!policy.exponential_backoff);
    assert_eq!(policy.max_retry_delay_ms, 0);
}

#[test]
fn retry_policy_new_has_sensible_defaults() {
    let policy = RetryPolicy::new(5);
    assert_eq!(policy.max_retries, 5);
    assert_eq!(policy.retry_delay_ms, 1000);
    assert!(!policy.exponential_backoff);
    assert_eq!(policy.max_retry_delay_ms, 30_000);
}

#[test]
fn retry_policy_builder_chain() {
    let policy = RetryPolicy::new(10)
        .with_delay(500)
        .with_exponential_backoff()
        .with_max_delay(5_000);

    assert_eq!(policy.max_retries, 10);
    assert_eq!(policy.retry_delay_ms, 500);
    assert!(policy.exponential_backoff);
    assert_eq!(policy.max_retry_delay_ms, 5_000);
}

#[test]
fn retry_policy_delay_for_attempt_linear_without_backoff() {
    let policy = RetryPolicy::new(3).with_delay(200);
    assert_eq!(policy.delay_for_attempt(0), 200);
    assert_eq!(policy.delay_for_attempt(1), 200);
    assert_eq!(policy.delay_for_attempt(5), 200);
    assert_eq!(policy.delay_for_attempt(100), 200);
}

#[test]
fn retry_policy_delay_for_attempt_exponential() {
    let policy = RetryPolicy::new(5)
        .with_delay(100)
        .with_exponential_backoff()
        .with_max_delay(10_000);

    assert_eq!(policy.delay_for_attempt(0), 100);
    assert_eq!(policy.delay_for_attempt(1), 200);
    assert_eq!(policy.delay_for_attempt(2), 400);
    assert_eq!(policy.delay_for_attempt(3), 800);
    assert_eq!(policy.delay_for_attempt(4), 1600);
    assert_eq!(policy.delay_for_attempt(5), 3200);
    assert_eq!(policy.delay_for_attempt(6), 6400);
    assert_eq!(policy.delay_for_attempt(7), 10_000);
}

#[test]
fn retry_policy_delay_for_attempt_capped_by_absolute_max() {
    let policy = RetryPolicy::new(100)
        .with_delay(u64::MAX)
        .with_exponential_backoff()
        .with_max_delay(u64::MAX);
    let delay = policy.delay_for_attempt(62);
    assert_eq!(delay, 3_600_000);
}

#[test]
fn retry_policy_delay_for_attempt_shift_capped_at_62() {
    let policy = RetryPolicy::new(100)
        .with_delay(1)
        .with_exponential_backoff()
        .with_max_delay(u64::MAX);
    let delay = policy.delay_for_attempt(62);
    assert_eq!(delay, 3_600_000, "delay capped by absolute max");
    let delay_63 = policy.delay_for_attempt(63);
    assert_eq!(delay_63, 3_600_000, "delay still capped by absolute max");
}

#[test]
fn retry_policy_should_retry_boundary_values() {
    let policy = RetryPolicy::new(3);
    assert!(policy.should_retry(0), "0 < 3");
    assert!(policy.should_retry(1), "1 < 3");
    assert!(policy.should_retry(2), "2 < 3");
    assert!(!policy.should_retry(3), "3 == 3, not < 3");
    assert!(!policy.should_retry(4), "4 > 3");
    assert!(!policy.should_retry(u32::MAX));
}

#[test]
fn retry_policy_should_retry_zero_max() {
    let policy = RetryPolicy::no_retries();
    assert!(!policy.should_retry(0), "0 retries allowed");
}

#[test]
fn retry_policy_serde_roundtrip() {
    assert_serde_roundtrip(&[
        RetryPolicy::new(5)
            .with_delay(200)
            .with_exponential_backoff()
            .with_max_delay(5000),
        RetryPolicy::no_retries(),
        RetryPolicy::default(),
    ]);
}

#[test]
fn retry_policy_equality() {
    let a = RetryPolicy::new(3).with_delay(100);
    let b = RetryPolicy::new(3).with_delay(100);
    let c = RetryPolicy::new(3).with_delay(200);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn refetch_trigger_default_is_always() {
    assert_eq!(RefetchTrigger::default(), RefetchTrigger::Always);
}

#[test]
fn refetch_trigger_labels() {
    assert_eq!(RefetchTrigger::Always.label(), "Always");
    assert_eq!(RefetchTrigger::IfStale.label(), "If stale");
    assert_eq!(RefetchTrigger::Never.label(), "Never");
}

#[test]
fn refetch_trigger_equality_and_copy() {
    let trigger = RefetchTrigger::IfStale;
    let copied = trigger;
    assert_eq!(trigger, copied);
    assert_ne!(trigger, RefetchTrigger::Always);
    assert_ne!(trigger, RefetchTrigger::Never);
}

#[test]
fn refetch_trigger_serde_roundtrip() {
    assert_serde_roundtrip(&[
        RefetchTrigger::Always,
        RefetchTrigger::IfStale,
        RefetchTrigger::Never,
    ]);
}
