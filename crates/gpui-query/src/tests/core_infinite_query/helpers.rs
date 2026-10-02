use crate::core::*;

pub fn make_resource() -> InfiniteQueryResource<Vec<&'static str>> {
    InfiniteQueryResource::new(
        QueryKey::from("items"),
        CachePolicy::Ttl { ttl_ms: 60_000 },
        RequestPolicy::LatestWins,
    )
}

pub fn make_bidirectional_resource() -> InfiniteQueryResource<Vec<&'static str>> {
    InfiniteQueryResource::new_bidirectional(
        QueryKey::from("items"),
        CachePolicy::Ttl { ttl_ms: 60_000 },
        RequestPolicy::LatestWins,
    )
}

const PAGE_LABELS: [&str; 64] = [
    "page0", "page1", "page2", "page3", "page4", "page5", "page6", "page7", "page8", "page9",
    "page10", "page11", "page12", "page13", "page14", "page15", "page16", "page17", "page18",
    "page19", "page20", "page21", "page22", "page23", "page24", "page25", "page26", "page27",
    "page28", "page29", "page30", "page31", "page32", "page33", "page34", "page35", "page36",
    "page37", "page38", "page39", "page40", "page41", "page42", "page43", "page44", "page45",
    "page46", "page47", "page48", "page49", "page50", "page51", "page52", "page53", "page54",
    "page55", "page56", "page57", "page58", "page59", "page60", "page61", "page62", "page63",
];

pub fn load_n_pages(n: usize) -> InfiniteQueryResource<Vec<&'static str>> {
    let mut r = make_resource();
    let mut seq = RequestSequencer::new();
    for (i, label) in PAGE_LABELS.iter().enumerate().take(n) {
        let has_more = i < n - 1;
        let id = r.begin_fetch_next(&mut seq, (i * 100) as u64).unwrap();
        r.complete_page_success(id, vec![*label], has_more, true, ((i + 1) * 100) as u64);
    }
    r
}
