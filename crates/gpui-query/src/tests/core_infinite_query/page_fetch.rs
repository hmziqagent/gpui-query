use super::helpers::*;
use crate::core::*;

#[test]
fn fetch_next_page_appends_page_and_updates_status() {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();

    let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    assert_eq!(r.status(), QueryStatus::LoadingEmpty);
    assert!(r.is_fetching_next_page());
    assert!(r.signal().is_some());

    let accepted = r.complete_page_success(id, vec!["a", "b"], true, true, 2_000);
    assert!(accepted);
    assert_eq!(r.page_count(), 1);
    assert_eq!(r.status(), QueryStatus::Success);
    assert_eq!(r.last_page(), Some(&vec!["a", "b"]));
    assert_eq!(r.last_updated_at_ms(), Some(2_000));
    assert!(!r.is_fetching_next_page());
    assert!(r.signal().is_none());
}

#[test]
fn fetch_next_page_accumulates_multiple_pages() {
    let r = load_n_pages(5);

    assert_eq!(r.page_count(), 5);
    assert_eq!(r.first_page(), Some(&vec!["page0"]));
    assert_eq!(r.last_page(), Some(&vec!["page4"]));
    assert!(!r.has_next_page());
}

#[test]
fn fetch_next_page_returns_none_when_no_next_page() {
    let mut r = make_resource();
    r.set_has_next_page(false);
    let mut seq = RequestSequencer::new();

    let id = r.begin_fetch_next(&mut seq, 1_000);
    assert!(id.is_none());
}

#[test]
fn fetch_previous_page_prepends_page() {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();

    let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    r.complete_page_success(id1, vec!["page1"], true, true, 2_000);

    r.set_has_previous_page(true);
    assert!(!r.is_fetching_previous_page());
    let id2 = r.begin_fetch_previous(&mut seq, 3_000).unwrap();
    assert!(r.is_fetching_previous_page());

    let accepted = r.complete_page_success(id2, vec!["page0"], false, false, 4_000);
    assert!(accepted);
    assert_eq!(r.page_count(), 2);
    assert_eq!(r.first_page(), Some(&vec!["page0"]));
    assert_eq!(r.last_page(), Some(&vec!["page1"]));
    assert!(!r.has_previous_page());
}

#[test]
fn fetch_previous_page_returns_none_when_no_previous_page() {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();
    let id = r.begin_fetch_previous(&mut seq, 1_000);
    assert!(id.is_none());
}

#[test]
fn begin_fetch_next_cancels_previous_signal() {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();

    let _id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    let old_signal = r.signal().unwrap().clone();
    assert!(!old_signal.is_cancelled());

    let _id2 = r.begin_fetch_next(&mut seq, 2_000).unwrap();
    assert!(old_signal.is_cancelled());
    assert!(!r.signal().unwrap().is_cancelled());
    assert_ne!(r.signal().unwrap(), &old_signal);
}

#[test]
fn begin_fetch_previous_cancels_previous_signal() {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();

    let _id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    let old_signal = r.signal().unwrap().clone();
    assert!(!old_signal.is_cancelled());

    r.set_has_previous_page(true);
    let _id2 = r.begin_fetch_previous(&mut seq, 2_000).unwrap();
    assert!(old_signal.is_cancelled());
    assert!(!r.is_fetching_next_page());
    assert!(r.is_fetching_previous_page());
}

#[test]
fn latest_wins_allows_replacement() {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();

    let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    let id2 = r.begin_fetch_next(&mut seq, 2_000).unwrap();

    assert_ne!(id1, id2);
    assert_eq!(r.cancelled_count(), 1);
}

#[test]
fn ignore_while_loading_prevents_replacement() {
    let mut r = InfiniteQueryResource::<Vec<&'static str>>::new(
        QueryKey::from("items"),
        CachePolicy::Ttl { ttl_ms: 60_000 },
        RequestPolicy::IgnoreWhileLoading,
    );
    let mut seq = RequestSequencer::new();

    let _id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    let id2 = r.begin_fetch_next(&mut seq, 2_000);
    assert!(id2.is_none());
    assert_eq!(r.cancelled_count(), 0);
}

#[test]
fn loading_empty_when_no_pages_exist() {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();

    let _id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    assert_eq!(r.status(), QueryStatus::LoadingEmpty);
    assert!(r.is_loading());
}

#[test]
fn loading_with_data_when_pages_exist() {
    let mut r = load_n_pages(1);
    let mut seq = RequestSequencer::new();

    r.set_has_next_page(true);

    let _id = r.begin_fetch_next(&mut seq, 5_000).unwrap();
    assert_eq!(r.status(), QueryStatus::LoadingWithData);
    assert!(r.is_loading());
}

#[test]
fn has_more_false_stops_further_fetches() {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();

    let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    r.complete_page_success(id, vec!["only"], false, true, 2_000);

    assert!(!r.has_next_page());
    let id2 = r.begin_fetch_next(&mut seq, 3_000);
    assert!(id2.is_none());
}

#[test]
fn has_more_propagated_on_prepend() {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();

    let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    r.complete_page_success(id1, vec!["page1"], true, true, 2_000);

    r.set_has_previous_page(true);
    let id2 = r.begin_fetch_previous(&mut seq, 3_000).unwrap();
    r.complete_page_success(id2, vec!["page0"], false, false, 4_000);

    assert!(!r.has_previous_page());
}

#[test]
fn has_more_true_on_prepend_preserves_has_previous() {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();

    let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    r.complete_page_success(id1, vec!["page1"], true, true, 2_000);

    r.set_has_previous_page(true);

    let id2 = r.begin_fetch_previous(&mut seq, 3_000).unwrap();
    r.complete_page_success(id2, vec!["page0"], true, false, 4_000);

    assert!(
        r.has_previous_page(),
        "has_more=true should keep has_previous_page=true"
    );
    assert_eq!(r.page_count(), 2);
    assert_eq!(r.first_page(), Some(&vec!["page0"]));
}

#[test]
fn ignore_while_loading_prevents_previous_page_replacement() {
    let mut r = InfiniteQueryResource::<Vec<&'static str>>::new(
        QueryKey::from("items"),
        CachePolicy::Ttl { ttl_ms: 60_000 },
        RequestPolicy::IgnoreWhileLoading,
    );
    let mut seq = RequestSequencer::new();
    r.set_has_previous_page(true);

    let _id1 = r.begin_fetch_previous(&mut seq, 1_000).unwrap();
    assert!(r.is_fetching_previous_page());

    let id2 = r.begin_fetch_previous(&mut seq, 2_000);
    assert!(
        id2.is_none(),
        "second begin_fetch_previous should be ignored"
    );
    assert_eq!(r.cancelled_count(), 0, "no cancellation on ignore");
}

#[test]
fn ignore_while_loading_allows_cross_direction_fetch() {
    let mut r = InfiniteQueryResource::<Vec<&'static str>>::new_bidirectional(
        QueryKey::from("items"),
        CachePolicy::Ttl { ttl_ms: 60_000 },
        RequestPolicy::IgnoreWhileLoading,
    );
    let mut seq = RequestSequencer::new();
    r.set_has_next_page(true);
    r.set_has_previous_page(true);

    let _id_next = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    assert!(r.is_fetching_next_page());

    let id_prev = r.begin_fetch_previous(&mut seq, 2_000);
    assert!(
        id_prev.is_some(),
        "cross-direction should succeed under IgnoreWhileLoading"
    );
    assert!(r.is_fetching_previous_page());
    assert!(!r.is_fetching_next_page());
}

#[test]
fn data_epoch_counts_page_writes_not_value_reads() {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();
    assert_eq!(r.data_epoch(), 0);

    let id = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    assert!(r.complete_page_success(id, vec!["a"], true, true, 2_000));
    assert_eq!(r.data_epoch(), 1);

    r.append_page(vec!["b"]);
    assert_eq!(r.data_epoch(), 2);
    r.prepend_page(vec!["c"]);
    assert_eq!(r.data_epoch(), 3);

    r.invalidate();
    assert_eq!(r.data_epoch(), 3, "invalidate is not a data write");

    r.set_max_pages(Some(1));
    assert_eq!(r.data_epoch(), 4, "evicting pages is a data change");
    r.set_max_pages(Some(10));
    assert_eq!(r.data_epoch(), 4, "no eviction is not a data write");

    r.reset();
    assert_eq!(r.data_epoch(), 5);
    r.reset();
    assert_eq!(r.data_epoch(), 5, "reset without pages is not a data write");
}

#[test]
fn equality_ignores_the_data_epoch() {
    let mut a = make_resource();
    a.set_max_pages(Some(1));
    a.append_page(vec!["a"]);
    let mut b = a.clone();
    assert_eq!(a, b);

    a.append_page(vec!["a"]);
    assert_eq!(
        a, b,
        "a page write that lands the same observable state must not split equality"
    );

    b.append_page(vec!["b"]);
    assert_ne!(a, b);
}
