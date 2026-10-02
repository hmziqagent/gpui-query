use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use proptest::prelude::*;

use crate::core::*;

pub fn arb_segments() -> impl Strategy<Value = Vec<String>> {
    prop_oneof![
        prop::collection::vec(any::<String>(), 1..10),
        prop::collection::vec(any::<String>(), 10..100),
    ]
}

pub fn arb_key_special() -> impl Strategy<Value = Vec<String>> {
    prop_oneof![
        any::<String>().prop_map(|s| vec![s]),
        prop::collection::vec("[\\p{L}\\p{N}\\p{P}\\p{S}]{1,20}", 1..5),
        any::<String>().prop_map(|s| vec![format!("{}::{}", s, s)]),
        prop::collection::vec(any::<String>(), 5..50),
        prop::collection::vec(arb_unicode_edge_case_string(), 1..5),
        ".{100,256}".prop_map(|s| vec![s]),
    ]
}

pub fn arb_unicode_edge_case_string() -> impl Strategy<Value = String> {
    use std::sync::LazyLock;
    static EDGE_CASES: LazyLock<Vec<String>> = LazyLock::new(|| {
        vec![
            "\u{200B}".to_string(),
            "\u{200C}".to_string(),
            "\u{200D}".to_string(),
            "\u{FEFF}".to_string(),
            "e\u{0301}".to_string(),
            "a\u{0308}\u{0301}".to_string(),
            "\u{0300}".to_string(),
            "\u{202A}".to_string(),
            "\u{202B}".to_string(),
            "\u{202C}".to_string(),
            "\u{202D}".to_string(),
            "\u{202E}".to_string(),
            "\u{2066}".to_string(),
            "\u{2067}".to_string(),
            "\u{2068}".to_string(),
            "\u{2069}".to_string(),
            "\u{00A0}".to_string(),
            "\u{FEFF}".to_string(),
            "\u{2000}".to_string(),
            "\u{3000}".to_string(),
            "\u{FFFD}".to_string(),
            "\u{00AD}".to_string(),
            "hello\u{202E}world\u{202C}".to_string(),
            "\u{0627}\u{0628}\u{062A}".to_string(),
            "\u{05D0}\u{05D1}\u{05D2}".to_string(),
            "\u{00E9}".to_string(),
            "e\u{0301}".to_string(),
            format!("x{}", "\u{0301}".repeat(50)),
            "\u{200B}\u{200C}\u{200D}\u{FEFF}".to_string(),
        ]
    });

    let cases = EDGE_CASES.clone();
    (any::<bool>(), any::<String>()).prop_map(move |(prefix, extra)| {
        let idx = (extra.len()) % cases.len();
        let edge = cases[idx].clone();
        if prefix {
            format!("{}{}", edge, extra)
        } else {
            format!("{}{}", extra, edge)
        }
    })
}

pub fn arb_key() -> impl Strategy<Value = Vec<String>> {
    prop_oneof![arb_segments(), arb_key_special()]
}

pub fn make_key(segments: &[String]) -> QueryKey {
    QueryKey::new(segments.iter().map(|s| s.as_str()))
}

pub fn hash_of(key: &QueryKey) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

pub fn assert_key_invariants(key: &QueryKey, expected_path: &str) {
    let cloned = key.clone();
    assert_eq!(key, &cloned, "clone should be equal");

    assert_eq!(
        hash_of(key),
        hash_of(&cloned),
        "hash should match for equal keys"
    );

    let json = serde_json::to_string(key).unwrap();
    let back: QueryKey = serde_json::from_str(&json).unwrap();
    assert_eq!(key, &back, "serde roundtrip should produce equal key");

    assert_eq!(
        key.to_path(),
        expected_path,
        "to_path should match expected"
    );
}
