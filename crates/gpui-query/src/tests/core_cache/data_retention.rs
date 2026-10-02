use crate::core::*;
use crate::tests::core_cache::*;

#[test]
fn previous_data_tracked_across_successive_successes() {
    let mut r = ttl_resource();
    seed_data(&mut r, "first", 100);
    assert_eq!(r.previous_data(), None, "no previous on first success");
    seed_data(&mut r, "second", 200);
    assert_eq!(r.data(), Some(&"second"));
    assert_eq!(r.previous_data(), Some(&"first"));
}

#[test]
fn previous_data_preserved_across_failure() {
    let mut r = ttl_resource();
    seed_data(&mut r, "v1", 100);
    seed_data(&mut r, "v2", 200);
    r.apply_failure("error", 300);
    assert_eq!(r.data(), Some(&"v2"), "failure preserves current data");
    assert_eq!(
        r.previous_data(),
        Some(&"v1"),
        "failure does not touch previous_data"
    );
}

#[test]
fn rollback_restores_previous_data() {
    let mut r = ttl_resource();
    seed_data(&mut r, "original", 100);
    seed_data(&mut r, "updated", 200);
    let rolled_back = r.rollback_to_previous();
    assert!(rolled_back);
    assert_eq!(r.data(), Some(&"original"));
    assert_eq!(r.status(), QueryStatus::Success);
    assert_eq!(
        r.previous_data(),
        None,
        "previous_data cleared after rollback"
    );
}

#[test]
fn rollback_returns_false_when_no_previous() {
    let mut r = ttl_resource();
    seed_data(&mut r, "only", 100);
    assert!(!r.rollback_to_previous());
    assert_eq!(r.data(), Some(&"only"));
}

#[test]
fn set_data_optimistic_update_saves_previous() {
    let mut r = ttl_resource();
    seed_data(&mut r, "original", 100);
    r.set_data("optimistic");
    assert_eq!(r.data(), Some(&"optimistic"));
    assert_eq!(r.previous_data(), Some(&"original"));
}

#[test]
fn clear_data_saves_to_previous() {
    let mut r = ttl_resource();
    seed_data(&mut r, "existing", 100);
    r.clear_data();
    assert_eq!(r.data(), None);
    assert_eq!(r.previous_data(), Some(&"existing"));
}

#[test]
fn rollback_after_optimistic_update() {
    let mut r = ttl_resource();
    seed_data(&mut r, "original", 100);
    r.set_data("optimistic");
    let rolled_back = r.rollback_to_previous();
    assert!(rolled_back);
    assert_eq!(r.data(), Some(&"original"));
    assert_eq!(r.status(), QueryStatus::Success);
}

#[test]
fn reset_clears_previous_data() {
    let mut r = ttl_resource();
    seed_data(&mut r, "first", 100);
    seed_data(&mut r, "second", 200);
    assert_eq!(r.previous_data(), Some(&"first"));
    r.reset();
    assert_eq!(r.previous_data(), None);
    assert_eq!(r.data(), None);
}
