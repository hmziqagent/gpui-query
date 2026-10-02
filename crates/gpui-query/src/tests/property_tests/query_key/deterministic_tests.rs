use crate::core::*;

use super::strategies::*;

#[test]
fn key_join_appends_and_chains_segments() {
    let key = QueryKey::from(["users"]).join("42");
    assert_eq!(key.parts().len(), 2);
    assert_eq!(key.to_path(), "users::42");

    let chained = key.join("posts");
    assert_eq!(chained.parts().len(), 3);
    assert_eq!(chained.to_path(), "users::42::posts");
    assert_eq!(key.parts().len(), 2, "join must not mutate the receiver");
}

#[test]
fn key_from_vec_string() {
    let key = QueryKey::from(vec!["users".to_string(), "42".to_string()]);
    assert_eq!(key.parts().len(), 2);
    assert_eq!(key.to_path(), "users::42");
}

#[test]
fn key_deref_allows_indexing_and_len() {
    let key = QueryKey::from(["a", "b", "c"]);
    assert_eq!(&*key[0], "a");
    assert_eq!(&*key[2], "c");
    assert_eq!(key.len(), 3);
}

#[test]
fn key_deserialize_accepts_single_string() {
    let key: QueryKey = serde_json::from_str("\"users\"").unwrap();
    assert_eq!(key.parts().len(), 1);
    assert_eq!(key.first_segment(), "users");
}

#[test]
fn key_deserialize_rejects_malformed_shapes() {
    for json in ["null", "42", "[\"a\", 1]", "[[\"a\"]]", "[]", "{}"] {
        let result: Result<QueryKey, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "malformed input {json} must Err, not panic or accept"
        );
    }
}

#[test]
fn key_empty_string_segment_distinguishes_from_multi() {
    let single_empty = QueryKey::from([""]);
    let two_empty = QueryKey::from(["", ""]);
    assert_ne!(single_empty, two_empty);
    assert_eq!(single_empty.to_path(), "");
    assert_eq!(two_empty.to_path(), "::");
}

#[test]
fn key_single_empty_segment_properties() {
    let key = QueryKey::from([""]);
    assert_eq!(key.parts().len(), 1);
    assert_eq!(key.first_segment(), "");
    assert_eq!(key.to_path(), "");
    assert_eq!(key.as_single(), Some(""));
}

#[test]
fn key_zero_width_space_segment() {
    let key = QueryKey::from(["\u{200B}"]);
    assert_key_invariants(&key, "\u{200B}");
}

#[test]
fn key_zero_width_joiner_segment() {
    let key = QueryKey::from(["\u{200D}"]);
    assert_key_invariants(&key, "\u{200D}");
}

#[test]
fn key_zwnj_segment() {
    let key = QueryKey::from(["\u{200C}"]);
    assert_key_invariants(&key, "\u{200C}");
}

#[test]
fn key_bom_segment() {
    let key = QueryKey::from(["\u{FEFF}"]);
    assert_key_invariants(&key, "\u{FEFF}");
}

#[test]
fn key_combining_acute_accent_nfd() {
    let nfd = "e\u{0301}";
    let key = QueryKey::from([nfd]);
    assert_key_invariants(&key, nfd);
}

#[test]
fn key_combining_precomposed_nfc() {
    let nfc = "\u{00E9}";
    let nfd = "e\u{0301}";
    let key_nfc = QueryKey::from([nfc]);
    let key_nfd = QueryKey::from([nfd]);
    assert_ne!(key_nfc, key_nfd, "NFC and NFD keys should be distinct");
    assert_key_invariants(&key_nfc, nfc);
    assert_key_invariants(&key_nfd, nfd);
}

#[test]
fn key_standalone_combining_mark() {
    let key = QueryKey::from(["\u{0300}"]);
    assert_key_invariants(&key, "\u{0300}");
}

#[test]
fn key_long_combining_chain() {
    let segment = format!("a{}", "\u{0301}".repeat(50));
    let key = QueryKey::from([&*segment]);
    assert_key_invariants(&key, &segment);
}

#[test]
fn key_rtl_override() {
    let key = QueryKey::from(["\u{202E}hello\u{202C}"]);
    assert_key_invariants(&key, "\u{202E}hello\u{202C}");
}

#[test]
fn key_bidi_isolates() {
    let key = QueryKey::from(["\u{2066}\u{2067}\u{2068}\u{2069}"]);
    assert_key_invariants(&key, "\u{2066}\u{2067}\u{2068}\u{2069}");
}

#[test]
fn key_replacement_character() {
    let key = QueryKey::from(["\u{FFFD}"]);
    assert_key_invariants(&key, "\u{FFFD}");
}

#[test]
fn key_soft_hyphen() {
    let key = QueryKey::from(["\u{00AD}"]);
    assert_key_invariants(&key, "\u{00AD}");
}

#[test]
fn key_non_breaking_space() {
    let key = QueryKey::from(["\u{00A0}"]);
    assert_key_invariants(&key, "\u{00A0}");
}

#[test]
fn key_ideographic_space() {
    let key = QueryKey::from(["\u{3000}"]);
    assert_key_invariants(&key, "\u{3000}");
}

#[test]
fn key_mixed_rtl_and_ltr() {
    let segment = "hello\u{0627}\u{0628}\u{062A}world";
    let key = QueryKey::from([segment]);
    assert_key_invariants(&key, segment);
}

#[test]
fn key_only_zero_width_chars_segment() {
    let segment = "\u{200B}\u{200C}\u{200D}\u{FEFF}";
    let key = QueryKey::from([segment]);
    assert_key_invariants(&key, segment);
}

#[test]
#[ignore = "stress: 2000-char single segment — run with --ignored"]
fn key_very_long_single_segment() {
    let segment = "x".repeat(2000);
    let key = QueryKey::from([&*segment]);
    assert_key_invariants(&key, &segment);
    assert_eq!(key.parts()[0].len(), 2000);
}

#[test]
fn key_unicode_edge_case_in_multi_segment_key() {
    let key = QueryKey::from(["\u{200B}", "\u{FEFF}", "\u{FFFD}"]);
    let expected_path = "\u{200B}::\u{FEFF}::\u{FFFD}";
    assert_key_invariants(&key, expected_path);
    let prefix = QueryKey::from(["\u{200B}"]);
    assert!(key.starts_with(&prefix));
    let prefix2 = QueryKey::from(["\u{200B}", "\u{FEFF}"]);
    assert!(key.starts_with(&prefix2));
    assert!(!prefix.starts_with(&key));
}

#[test]
#[ignore = "stress: 200-segment key — run with --ignored"]
fn key_deeply_nested_200_segments() {
    let segments: Vec<String> = (0..200).map(|i| format!("seg{}", i)).collect();
    let key = make_key(&segments);
    assert_key_invariants(&key, &segments.join("::"));
    for depth in [1, 50, 100, 199] {
        let prefix_segs: Vec<String> = segments[..depth].to_vec();
        let prefix = make_key(&prefix_segs);
        assert!(
            key.starts_with(&prefix),
            "should match prefix of depth {}",
            depth
        );
    }
}

#[test]
fn key_unicode_edge_cases_distinct_keys() {
    let zwsp = QueryKey::from(["\u{200B}"]);
    let zwnj = QueryKey::from(["\u{200C}"]);
    let zwj = QueryKey::from(["\u{200D}"]);
    let bom = QueryKey::from(["\u{FEFF}"]);

    assert_ne!(zwsp, zwnj);
    assert_ne!(zwsp, zwj);
    assert_ne!(zwsp, bom);
    assert_ne!(zwnj, zwj);
    assert_ne!(zwnj, bom);
    assert_ne!(zwj, bom);

    let hashes: Vec<u64> = [&zwsp, &zwnj, &zwj, &bom]
        .iter()
        .map(|k| hash_of(k))
        .collect();
    let unique_hashes: std::collections::HashSet<u64> = hashes.into_iter().collect();
    assert_eq!(
        unique_hashes.len(),
        4,
        "four distinct zero-width chars should have distinct hashes"
    );
}
