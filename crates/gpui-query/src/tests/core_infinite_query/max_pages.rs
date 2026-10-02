use super::helpers::*;
use crate::core::*;

#[test]
fn max_pages_evicts_oldest_page_on_append() {
    let mut r = make_resource();
    r.set_max_pages(Some(2));
    let mut seq = RequestSequencer::new();

    let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    r.complete_page_success(id1, vec!["a"], true, true, 2_000);
    let id2 = r.begin_fetch_next(&mut seq, 3_000).unwrap();
    r.complete_page_success(id2, vec!["b"], true, true, 4_000);

    let id3 = r.begin_fetch_next(&mut seq, 5_000).unwrap();
    r.complete_page_success(id3, vec!["c"], false, true, 6_000);

    assert_eq!(r.page_count(), 2);
    assert_eq!(r.first_page(), Some(&vec!["b"]));
    assert_eq!(r.last_page(), Some(&vec!["c"]));
}

#[test]
fn max_pages_evicts_newest_page_on_prepend() {
    let mut r = make_resource();
    r.set_max_pages(Some(2));
    let mut seq = RequestSequencer::new();

    let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    r.complete_page_success(id1, vec!["a"], true, true, 2_000);
    let id2 = r.begin_fetch_next(&mut seq, 3_000).unwrap();
    r.complete_page_success(id2, vec!["b"], true, true, 4_000);

    r.set_has_previous_page(true);
    let id3 = r.begin_fetch_previous(&mut seq, 5_000).unwrap();
    r.complete_page_success(id3, vec!["c"], false, false, 6_000);

    assert_eq!(r.page_count(), 2);
    assert_eq!(r.pages()[0].as_ref(), &vec!["c"]);
    assert_eq!(r.pages()[1].as_ref(), &vec!["a"]);
}

#[test]
fn max_pages_zero_treated_as_unbounded() {
    let mut r = load_n_pages(3);

    r.set_max_pages(Some(0));
    assert_eq!(r.max_pages(), None);
    assert_eq!(r.page_count(), 3);
}

#[test]
fn max_pages_one_retains_only_latest_page() {
    let mut r = make_resource();
    r.set_max_pages(Some(1));
    let mut seq = RequestSequencer::new();

    let id1 = r.begin_fetch_next(&mut seq, 1_000).unwrap();
    r.complete_page_success(id1, vec!["a"], true, true, 2_000);
    let id2 = r.begin_fetch_next(&mut seq, 3_000).unwrap();
    r.complete_page_success(id2, vec!["b"], true, true, 4_000);

    assert_eq!(r.page_count(), 1);
    assert_eq!(r.first_page(), Some(&vec!["b"]));
    assert_eq!(r.last_page(), Some(&vec!["b"]));
}

#[test]
fn max_pages_default_is_50() {
    let r = make_resource();
    assert_eq!(r.max_pages(), Some(50));
}

#[test]
fn max_pages_50_allows_50_pages_and_evicts_on_51st() {
    let mut r = make_resource();
    assert_eq!(r.max_pages(), Some(50));
    let mut seq = RequestSequencer::new();

    const P_LABELS: [&str; 51] = [
        "p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9", "p10", "p11", "p12", "p13",
        "p14", "p15", "p16", "p17", "p18", "p19", "p20", "p21", "p22", "p23", "p24", "p25", "p26",
        "p27", "p28", "p29", "p30", "p31", "p32", "p33", "p34", "p35", "p36", "p37", "p38", "p39",
        "p40", "p41", "p42", "p43", "p44", "p45", "p46", "p47", "p48", "p49", "p50",
    ];

    for (i, label) in P_LABELS.iter().enumerate().take(50) {
        let id = r.begin_fetch_next(&mut seq, (i * 100) as u64).unwrap();
        r.complete_page_success(id, vec![*label], true, true, ((i + 1) * 100) as u64);
    }
    assert_eq!(r.page_count(), 50);
    assert_eq!(r.first_page(), Some(&vec!["p0"]));

    let id51 = r.begin_fetch_next(&mut seq, 5_000_000).unwrap();
    r.complete_page_success(id51, vec!["p50"], false, true, 5_000_100);
    assert_eq!(r.page_count(), 50);
    assert_eq!(r.first_page(), Some(&vec!["p1"]));
    assert_eq!(r.last_page(), Some(&vec!["p50"]));
}

#[test]
fn set_max_pages_returns_evicted_pages() {
    let mut r = load_n_pages(3);

    let evicted = r.set_max_pages(Some(2));
    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted[0].as_ref(), &vec!["page0"]);
    assert_eq!(r.page_count(), 2);
}

#[test]
fn append_page_returns_evicted_pages() {
    let mut r = make_resource();
    r.set_max_pages(Some(2));

    assert!(r.append_page(vec!["a"]).is_empty());
    assert!(r.append_page(vec!["b"]).is_empty());

    let evicted = r.append_page(vec!["c"]);
    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted[0].as_ref(), &vec!["a"]);
    assert_eq!(r.page_count(), 2);
}

#[test]
fn prepend_page_returns_evicted_pages() {
    let mut r = make_resource();
    r.set_max_pages(Some(2));

    r.prepend_page(vec!["a"]);
    r.prepend_page(vec!["b"]);

    let evicted = r.prepend_page(vec!["c"]);
    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted[0].as_ref(), &vec!["a"]);
    assert_eq!(r.page_count(), 2);
    assert_eq!(r.first_page(), Some(&vec!["c"]));
}
