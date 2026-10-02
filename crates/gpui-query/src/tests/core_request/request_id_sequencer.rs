use crate::core::{RequestId, RequestSequencer};
use std::num::NonZero;

#[test]
fn request_id_scoped_accesses_scope_and_value() {
    let id = RequestId::scoped(NonZero::new(3).unwrap(), 7);
    assert_eq!(id.scope_id(), NonZero::new(3).unwrap());
    assert_eq!(id.value(), 7);
}

#[test]
fn request_id_label_format() {
    let id = RequestId::scoped(NonZero::new(42).unwrap(), 99);
    assert_eq!(id.label(), "42:99");
}

#[test]
fn request_id_equality_requires_both_fields() {
    let a = RequestId::scoped(NonZero::new(1).unwrap(), 10);
    let b = RequestId::scoped(NonZero::new(1).unwrap(), 10);
    let c = RequestId::scoped(NonZero::new(2).unwrap(), 10);
    let d = RequestId::scoped(NonZero::new(1).unwrap(), 20);
    assert_eq!(a, b, "same scope and sequence should be equal");
    assert_ne!(a, c, "different scope should not be equal");
    assert_ne!(a, d, "different sequence should not be equal");
}

#[test]
fn request_id_ordering_is_lexicographic() {
    let a = RequestId::scoped(NonZero::new(1).unwrap(), 100);
    let b = RequestId::scoped(NonZero::new(2).unwrap(), 1);
    let c = RequestId::scoped(NonZero::new(1).unwrap(), 200);
    assert!(a < b, "scope 1 < scope 2 regardless of sequence");
    assert!(a < c, "scope 1 seq 100 < scope 1 seq 200");
}

#[test]
fn sequencer_starts_at_scope_1_seq_1() {
    let mut seq = RequestSequencer::new();
    let id = seq.next_request();
    assert_eq!(id.scope_id(), NonZero::new(1).unwrap());
    assert_eq!(id.value(), 1);
}

#[test]
fn sequencer_produces_monotonically_increasing_ids() {
    let mut seq = RequestSequencer::new();
    let mut prev = seq.next_request();
    for _ in 0..50 {
        let curr = seq.next_request();
        assert!(
            curr > prev,
            "expected {:?} > {:?} but ordering violated",
            curr,
            prev
        );
        prev = curr;
    }
}

#[test]
fn sequencer_default_matches_new() {
    let default = RequestSequencer::default();
    let new = RequestSequencer::new();
    assert_eq!(default, new);
}

#[test]
fn sequencer_is_current_scope_tracks_scope_changes() {
    let mut seq = RequestSequencer::new();
    let id_in_scope = seq.next_request();
    assert!(seq.is_current_scope(id_in_scope));

    seq.advance_scope();
    assert!(
        !seq.is_current_scope(id_in_scope),
        "after advance_scope, old ids should not match"
    );

    let id_in_new_scope = seq.next_request();
    assert!(seq.is_current_scope(id_in_new_scope));
}

#[test]
fn sequencer_advances_scope_when_sequence_reaches_max() {
    let mut seq = RequestSequencer {
        scope_id: NonZero::new(5).unwrap(),
        next_request_id: u64::MAX,
    };
    let id = seq.next_request();
    assert_eq!(id.scope_id(), NonZero::new(5).unwrap());
    assert_eq!(id.value(), u64::MAX);

    let next_id = seq.next_request();
    assert_eq!(
        next_id.scope_id(),
        NonZero::new(6).unwrap(),
        "scope should have advanced to 6"
    );
    assert_eq!(
        next_id.value(),
        1,
        "sequence should reset to 1 after scope advance"
    );
}

#[test]
fn sequencer_scope_id_wraps_to_1_on_overflow() {
    let mut seq = RequestSequencer {
        scope_id: NonZero::new(u64::MAX).unwrap(),
        next_request_id: u64::MAX,
    };
    let id = seq.next_request();
    assert_eq!(id.scope_id(), NonZero::new(u64::MAX).unwrap());
    assert_eq!(id.value(), u64::MAX);

    let next_id = seq.next_request();
    assert_eq!(
        next_id.scope_id(),
        NonZero::new(1).unwrap(),
        "scope_id should wrap to 1 on u64::MAX overflow"
    );
    assert_eq!(next_id.value(), 1);
}
