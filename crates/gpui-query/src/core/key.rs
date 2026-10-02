use std::ops::Deref;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeSeq};

/// Hierarchical key (`["todos", id]` style). Must contain at least one
/// segment: [`QueryKey::new`](Self::new) panics on empty, serde returns an
/// error instead. Cloning is one refcount bump (`Arc<[Arc<str>]>`).
///
/// # Examples
///
/// ```
/// use gpui_query::QueryKey;
///
/// let key = QueryKey::from(["users", "42", "posts"]);
/// assert!(key.starts_with(&QueryKey::from(["users"])));
/// assert!(!key.starts_with(&QueryKey::from(["posts"])));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueryKey(Arc<[Arc<str>]>);

impl QueryKey {
    /// # Panics
    ///
    /// If the iterator yields zero segments.
    pub fn new(parts: impl IntoIterator<Item: AsRef<str>>) -> Self {
        let segments: Vec<Arc<str>> = parts.into_iter().map(|s| Arc::from(s.as_ref())).collect();
        assert!(
            !segments.is_empty(),
            "QueryKey must contain at least one segment"
        );
        Self(segments.into())
    }

    #[must_use]
    pub fn from_single(value: impl AsRef<str>) -> Self {
        Self(Arc::from([Arc::from(value.as_ref())]))
    }

    #[must_use]
    pub fn parts(&self) -> &[Arc<str>] {
        &self.0
    }

    #[must_use]
    pub fn as_single(&self) -> Option<&str> {
        if self.0.len() == 1 {
            Some(&self.0[0])
        } else {
            None
        }
    }

    #[must_use]
    pub fn first_segment(&self) -> &str {
        match self.0.first() {
            Some(s) => s.as_ref(),
            None => "",
        }
    }

    /// Joins with `"::"`, escaping `\` and `:` inside segments so distinct
    /// keys never map to one path; `from_path` inverts it.
    pub fn to_path(&self) -> String {
        const SEP: &str = "::";
        let escapes: usize = self
            .0
            .iter()
            .map(|s| s.chars().filter(|c| matches!(c, '\\' | ':')).count())
            .sum();
        let len = self.0.iter().map(|s| s.len()).sum::<usize>()
            + escapes
            + SEP.len() * self.0.len().saturating_sub(1);
        let mut path = String::with_capacity(len);
        for (i, segment) in self.0.iter().enumerate() {
            if i > 0 {
                path.push_str(SEP);
            }
            for ch in segment.chars() {
                if matches!(ch, '\\' | ':') {
                    path.push('\\');
                }
                path.push(ch);
            }
        }
        path
    }

    /// Inverse of [`to_path`](Self::to_path); any input yields at least one
    /// segment and never panics.
    #[cfg(any(feature = "persist", test))]
    pub(crate) fn from_path(path: &str) -> Self {
        let mut segments: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut escaped = false;
        let mut chars = path.chars();
        while let Some(ch) = chars.next() {
            if escaped {
                current.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == ':' {
                segments.push(std::mem::take(&mut current));
                match chars.next() {
                    Some(':') => {}
                    Some('\\') => escaped = true,
                    Some(other) => current.push(other),
                    None => {}
                }
            } else {
                current.push(ch);
            }
        }
        segments.push(current);
        Self::new(segments)
    }

    /// An empty `prefix` matches every valid key; an empty `self` never matches.
    pub fn starts_with(&self, prefix: &QueryKey) -> bool {
        if prefix.0.is_empty() {
            return !self.0.is_empty();
        }
        self.0.starts_with(&prefix.0)
    }

    /// O(n) copy of all segments; prefer building the full key in one
    /// [`new`](Self::new) call over chaining `join`s.
    pub fn join(&self, extra: &str) -> QueryKey {
        let mut parts: Vec<Arc<str>> = self.0.to_vec();
        parts.push(Arc::from(extra));
        Self(parts.into())
    }
}

impl Deref for QueryKey {
    type Target = [Arc<str>];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<&str> for QueryKey {
    fn from(value: &str) -> Self {
        Self::from_single(value)
    }
}

impl From<String> for QueryKey {
    fn from(value: String) -> Self {
        Self::from_single(value)
    }
}

impl<const N: usize> From<[&str; N]> for QueryKey {
    fn from(parts: [&str; N]) -> Self {
        Self::new(parts)
    }
}

impl From<Vec<&str>> for QueryKey {
    fn from(parts: Vec<&str>) -> Self {
        Self::new(parts)
    }
}

impl From<Vec<String>> for QueryKey {
    fn from(parts: Vec<String>) -> Self {
        Self::new(parts)
    }
}

impl Serialize for QueryKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for s in self.0.iter() {
            seq.serialize_element(s.as_ref())?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for QueryKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum KeyRepr {
            Array(Vec<String>),
            String(String),
        }

        match KeyRepr::deserialize(deserializer)? {
            // Deserializing untrusted input cannot panic, so empty must Err here.
            KeyRepr::Array(parts) if !parts.is_empty() => Ok(Self::new(parts)),
            KeyRepr::Array(_) => Err(serde::de::Error::custom(
                "QueryKey must contain at least one segment",
            )),
            KeyRepr::String(s) => Ok(Self::from_single(s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_part_key_from_str() {
        let key = QueryKey::from("users");
        assert_eq!(key.parts().len(), 1);
        assert_eq!(key.first_segment(), "users");
        assert_eq!(key.as_single(), Some("users"));
    }

    #[test]
    fn multi_part_key_from_array() {
        let key = QueryKey::from(["users", "42", "posts"]);
        assert_eq!(key.parts().len(), 3);
        assert_eq!(key.as_single(), None);
    }

    #[test]
    fn starts_with_prefix() {
        let key = QueryKey::from(["users", "42", "posts"]);
        assert!(key.starts_with(&QueryKey::from(["users"])));
        assert!(key.starts_with(&QueryKey::from(["users", "42"])));
        assert!(!key.starts_with(&QueryKey::from(["posts"])));
    }

    #[test]
    fn to_path_joins_segments() {
        let key = QueryKey::from(["users", "42", "posts"]);
        assert_eq!(key.to_path(), "users::42::posts");
    }

    #[test]
    fn clone_is_cheap() {
        let key = QueryKey::from(["a", "b", "c"]);
        let cloned = key.clone();
        assert_eq!(key, cloned);
        assert!(Arc::ptr_eq(&key.0, &cloned.0));
    }

    #[test]
    fn serde_roundtrip() {
        let key = QueryKey::from(["users", "42"]);
        let json = serde_json::to_string(&key).unwrap();
        let back: QueryKey = serde_json::from_str(&json).unwrap();
        assert_eq!(key, back);
    }

    #[test]
    fn serde_empty_array_returns_err_not_panic() {
        let err = serde_json::from_str::<QueryKey>("[]").unwrap_err();
        assert!(err.to_string().contains("at least one segment"));
    }

    #[test]
    fn serde_resource_empty_key_field_returns_err_not_panic() {
        use crate::core::{CachePolicy, QueryResource, RequestPolicy};

        let resource = QueryResource::<String>::new(
            QueryKey::from(["users"]),
            CachePolicy::NoCache,
            RequestPolicy::LatestWins,
        );
        let mut value = serde_json::to_value(&resource).unwrap();
        value["key"] = serde_json::Value::Array(Vec::new());
        let json = value.to_string();

        let err = serde_json::from_str::<QueryResource<String>>(&json).unwrap_err();
        assert!(err.to_string().contains("at least one segment"));
    }

    #[test]
    #[should_panic(expected = "QueryKey must contain at least one segment")]
    fn new_rejects_empty_parts() {
        let _ = QueryKey::new(std::iter::empty::<&str>());
    }

    #[test]
    fn starts_with_empty_prefix_matches_valid_key() {
        let key = QueryKey::from(["users", "42"]);
        let empty = QueryKey(Arc::from([] as [Arc<str>; 0]));
        assert!(key.starts_with(&empty));
    }

    #[test]
    fn starts_with_empty_prefix_does_not_match_empty_key() {
        let empty = QueryKey(Arc::from([] as [Arc<str>; 0]));
        assert!(!empty.starts_with(&empty));
    }

    #[test]
    fn to_path_disambiguates_forward_slash_in_segment() {
        let key_with_slash = QueryKey::from(["a/b"]);
        let key_two_parts = QueryKey::from(["a", "b"]);
        assert_ne!(key_with_slash.to_path(), key_two_parts.to_path());
        assert_eq!(key_with_slash.to_path(), "a/b");
        assert_eq!(key_two_parts.to_path(), "a::b");
    }

    #[test]
    fn to_path_disambiguates_empty_string_segments() {
        let key = QueryKey::from(["", ""]);
        assert_eq!(key.to_path(), "::");
    }

    #[test]
    fn to_path_escapes_colon_and_backslash_in_segments() {
        let key = QueryKey::from(["a\\b", "c:d"]);
        assert_eq!(key.to_path(), "a\\\\b::c\\:d");
    }

    #[test]
    fn to_path_collision_pair_maps_to_distinct_paths() {
        let first = QueryKey::from(["a::", ""]);
        let second = QueryKey::from(["a", "::"]);
        assert_ne!(first.to_path(), second.to_path());
        assert_eq!(first.to_path(), "a\\:\\:::");
        assert_eq!(second.to_path(), "a::\\:\\:");
    }

    #[test]
    fn from_path_inverts_to_path_for_escaped_segments() {
        for parts in [
            vec!["a::", ""],
            vec!["a", "::"],
            vec!["a\\b", "c:d"],
            vec!["::"],
            vec![""],
            vec!["\\"],
        ] {
            let key = QueryKey::from(parts);
            assert_eq!(QueryKey::from_path(&key.to_path()), key);
        }
    }

    #[test]
    fn from_path_hostile_input_yields_at_least_one_segment() {
        for path in ["", "::", "\\", "a\\", "a:b", "a:::b", "a:::", ":\\:"] {
            assert!(
                !QueryKey::from_path(path).parts().is_empty(),
                "from_path({path:?}) must yield at least one segment"
            );
        }
    }
}
