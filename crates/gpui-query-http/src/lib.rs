//! HTTP cache-header helpers for [`gpui_query`]: turn server cache headers
//! into a [`CachePolicy`] ("server wins") and layer an in-memory [`HttpCache`]
//! over any [`HttpBackend`]. Core-only dependency, any async runtime.
//!
//! # Server wins
//!
//! ```no_run
//! # use gpui_query_http::cache_policy_from_headers;
//! # use gpui_query::core::{CachePolicy, Fetched};
//! # fn handle(response_headers: &http::HeaderMap, data: String) {
//! let policy = cache_policy_from_headers(response_headers)
//!     .unwrap_or(CachePolicy::NoCache);
//! let fetched = Fetched::with_policy(data, policy);
//! # }
//! ```

#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::time::Duration;

use gpui_query::core::CachePolicy;
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod backend;
pub mod cache;
#[cfg(feature = "reqwest")]
#[cfg_attr(docsrs, doc(cfg(feature = "reqwest")))]
pub mod reqwest_backend;

pub use backend::{BackendResponse, Conditionals, HttpBackend, MaybeSend};
pub use cache::{HttpCache, HttpError};
#[cfg(feature = "reqwest")]
#[cfg_attr(docsrs, doc(cfg(feature = "reqwest")))]
pub use reqwest_backend::ReqwestBackend;

/// Cache metadata from a response, serializable (epoch-based) so a
/// persistence layer can rehydrate validators for cheap `304` refetches.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheMeta {
    /// Sent as `If-None-Match` on revalidation.
    pub etag: Option<String>,
    /// Sent as `If-Modified-Since` on revalidation.
    pub last_modified: Option<String>,
    /// When the entry was stored; freshness is measured from here.
    pub stored_at: std::time::SystemTime,
    /// Freshness window, from the policy's TTL.
    pub fresh_for: Duration,
    /// SWR window: how long a stale entry may serve while revalidating.
    pub stale_for: Duration,
}

/// Errors parsing cache headers into a [`CachePolicy`].
#[derive(Debug, Error)]
pub enum ParseError {
    /// `max-age` (or `s-maxage`) had a non-integer value.
    #[error("invalid max-age value: {0}")]
    InvalidMaxAge(String),
    /// `stale-while-revalidate` had a non-integer value.
    #[error("invalid stale-while-revalidate value: {0}")]
    InvalidStaleWhileRevalidate(String),
}

/// Derives a [`CachePolicy`] from response `Cache-Control` headers ("server
/// wins"): `no-store`/`no-cache` from any position wins (RFC 9111 §5.2.2),
/// otherwise the first `max-age` (`s-maxage` as private-cache fallback,
/// §5.2.2.10) with `stale-while-revalidate` if present; duplicates keep the
/// first occurrence (§4.2.1), over-large delta-seconds saturate (§1.2.2),
/// malformed values error as [`ParseError`].
pub fn cache_policy_from_headers(headers: &HeaderMap) -> Result<CachePolicy, ParseError> {
    // Option<Result>: first occurrence wins; errors surface only after the scan.
    let mut s_maxage: Option<Result<u64, ParseError>> = None;
    let mut max_age: Option<Result<u64, ParseError>> = None;
    let mut swr: Option<Result<u64, ParseError>> = None;

    for value in headers.get_all(http::header::CACHE_CONTROL).iter() {
        let Ok(raw) = value.to_str() else {
            continue;
        };
        for directive in split_cache_directives(raw) {
            let directive = directive.trim();
            if directive.is_empty() {
                continue;
            }
            let (name, val) = match directive.split_once('=') {
                Some((n, v)) => (n.trim(), Some(v.trim().trim_matches('"'))),
                None => (directive, None),
            };
            if name.eq_ignore_ascii_case("no-store") || name.eq_ignore_ascii_case("no-cache") {
                return Ok(CachePolicy::NoCache);
            }
            let is_swr = name.eq_ignore_ascii_case("stale-while-revalidate");
            let slot = if is_swr {
                Some(&mut swr)
            } else if name.eq_ignore_ascii_case("s-maxage") {
                Some(&mut s_maxage)
            } else if name.eq_ignore_ascii_case("max-age") {
                Some(&mut max_age)
            } else {
                None
            };
            if let Some(slot) = slot
                && slot.is_none()
                && let Some(v) = val
            {
                *slot = Some(parse_secs(is_swr, v));
            }
        }
    }

    let s_maxage_secs = s_maxage.transpose()?;
    let max_age_secs = max_age.transpose()?;
    let swr_secs = swr.transpose()?;

    match (max_age_secs.or(s_maxage_secs), swr_secs) {
        (Some(secs), Some(stale)) => Ok(CachePolicy::StaleWhileRevalidate {
            ttl_ms: secs.saturating_mul(1000),
            stale_ms: stale.saturating_mul(1000),
        }),
        (Some(secs), None) => Ok(CachePolicy::Ttl {
            ttl_ms: secs.saturating_mul(1000),
        }),
        (None, _) => Ok(CachePolicy::NoCache),
    }
}

/// Splits a `Cache-Control` value on commas outside quoted-strings, so a
/// quoted argument containing `,` cannot smuggle in extra directives.
fn split_cache_directives(raw: &str) -> impl Iterator<Item = &str> {
    let mut pos = 0;
    std::iter::from_fn(move || {
        if pos > raw.len() {
            return None;
        }
        let bytes = raw.as_bytes();
        let start = pos;
        let mut end = bytes.len();
        let mut quoted = false;
        let mut i = start;
        while i < bytes.len() {
            match bytes[i] {
                // Skip the character after a backslash inside a quoted-string.
                b'\\' if quoted => i += 1,
                b'"' => quoted = !quoted,
                b',' if !quoted => {
                    end = i;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        pos = if end < bytes.len() {
            end + 1
        } else {
            bytes.len() + 1
        };
        Some(&raw[start..end])
    })
}

// Errors echo attacker-controlled header bytes; bound them like core's sanitizer.
const ERROR_VALUE_MAX_BYTES: usize = 512;

fn parse_secs(is_stale: bool, raw: &str) -> Result<u64, ParseError> {
    if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(match raw.parse::<u128>() {
            Ok(n) => n.min(u64::MAX as u128) as u64,
            // Longer than u128: still all digits, still saturate.
            Err(_) => u64::MAX,
        });
    }
    let value = if raw.len() <= ERROR_VALUE_MAX_BYTES {
        raw.to_string()
    } else {
        let mut cut = ERROR_VALUE_MAX_BYTES;
        while !raw.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}...[truncated]", &raw[..cut])
    };
    Err(if is_stale {
        ParseError::InvalidStaleWhileRevalidate(value)
    } else {
        ParseError::InvalidMaxAge(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_query::core::CachePolicy;
    use http::HeaderMap;

    fn cc(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(http::header::CACHE_CONTROL, value.parse().unwrap());
        h
    }

    #[test]
    fn max_age_yields_ttl() {
        let policy = cache_policy_from_headers(&cc("max-age=600")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 600_000 });
    }

    #[test]
    fn max_age_and_swr_yields_swr() {
        let policy =
            cache_policy_from_headers(&cc("max-age=600, stale-while-revalidate=120")).unwrap();
        assert_eq!(
            policy,
            CachePolicy::StaleWhileRevalidate {
                ttl_ms: 600_000,
                stale_ms: 120_000
            }
        );
    }

    #[test]
    fn max_age_wins_over_s_maxage() {
        let policy = cache_policy_from_headers(&cc("max-age=10, s-maxage=30")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 10_000 });
    }

    #[test]
    fn s_maxage_alone_sets_ttl() {
        let policy = cache_policy_from_headers(&cc("s-maxage=30")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 30_000 });
    }

    #[test]
    fn no_store_wins_over_s_maxage() {
        assert_eq!(
            cache_policy_from_headers(&cc("s-maxage=30, no-store")).unwrap(),
            CachePolicy::NoCache
        );
    }

    #[test]
    fn no_store_yields_no_cache() {
        assert_eq!(
            cache_policy_from_headers(&cc("no-store")).unwrap(),
            CachePolicy::NoCache
        );
    }

    #[test]
    fn no_cache_directive_yields_no_cache() {
        assert_eq!(
            cache_policy_from_headers(&cc("no-cache")).unwrap(),
            CachePolicy::NoCache
        );
    }

    #[test]
    fn no_store_short_circuits_before_parsing_later_directives() {
        let policy = cache_policy_from_headers(&cc("no-store, max-age=abc")).unwrap();
        assert_eq!(policy, CachePolicy::NoCache);
    }

    #[test]
    fn no_store_wins_even_after_malformed_value() {
        let policy = cache_policy_from_headers(&cc("max-age=abc, no-store")).unwrap();
        assert_eq!(policy, CachePolicy::NoCache);
    }

    #[test]
    fn no_cache_control_yields_no_cache() {
        let policy = cache_policy_from_headers(&HeaderMap::new()).unwrap();
        assert_eq!(policy, CachePolicy::NoCache);
    }

    #[test]
    fn quoted_value_is_accepted() {
        let policy = cache_policy_from_headers(&cc("max-age=\"600\"")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 600_000 });
    }

    #[test]
    fn directive_names_are_case_insensitive() {
        let policy = cache_policy_from_headers(&cc("Max-Age=60")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 60_000 });
    }

    #[test]
    fn whitespace_around_equals_is_tolerated() {
        let policy = cache_policy_from_headers(&cc("max-age = 120")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 120_000 });
    }

    #[test]
    fn other_directives_are_ignored() {
        let policy = cache_policy_from_headers(&cc("public, max-age=5")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 5_000 });
    }

    #[test]
    fn quoted_comma_cannot_smuggle_directives() {
        let policy = cache_policy_from_headers(&cc("private=\"a, max-age=86400\"")).unwrap();
        assert_eq!(policy, CachePolicy::NoCache);
    }

    #[test]
    fn multiple_cache_control_headers_combine() {
        let mut h = HeaderMap::new();
        h.insert(http::header::CACHE_CONTROL, "max-age=10".parse().unwrap());
        h.append(
            http::header::CACHE_CONTROL,
            "stale-while-revalidate=20".parse().unwrap(),
        );
        let policy = cache_policy_from_headers(&h).unwrap();
        assert_eq!(
            policy,
            CachePolicy::StaleWhileRevalidate {
                ttl_ms: 10_000,
                stale_ms: 20_000
            }
        );
    }

    #[test]
    fn duplicate_directives_keep_first_occurrence() {
        let policy = cache_policy_from_headers(&cc("max-age=600, max-age=86400")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 600_000 });
    }

    #[test]
    fn digit_overflow_saturates_instead_of_erroring() {
        let policy = cache_policy_from_headers(&cc("max-age=99999999999999999999999")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: u64::MAX });
    }

    #[test]
    fn bare_max_age_without_value_is_ignored() {
        assert_eq!(
            cache_policy_from_headers(&cc("max-age")).unwrap(),
            CachePolicy::NoCache
        );
    }

    #[test]
    fn invalid_max_age_is_typed_error() {
        let err = cache_policy_from_headers(&cc("max-age=abc")).unwrap_err();
        assert!(matches!(err, ParseError::InvalidMaxAge(_)));
        assert!(err.to_string().contains("max-age"));
    }

    #[test]
    fn negative_max_age_is_typed_error() {
        let err = cache_policy_from_headers(&cc("max-age=-1")).unwrap_err();
        assert!(matches!(err, ParseError::InvalidMaxAge(_)));
    }

    #[test]
    fn invalid_swr_is_typed_error() {
        let err =
            cache_policy_from_headers(&cc("max-age=10, stale-while-revalidate=oops")).unwrap_err();
        assert!(matches!(err, ParseError::InvalidStaleWhileRevalidate(_)));
    }

    #[test]
    fn no_cache_with_field_argument_is_no_cache() {
        assert_eq!(
            cache_policy_from_headers(&cc("no-cache=\"Set-Cookie\"")).unwrap(),
            CachePolicy::NoCache
        );
    }

    #[test]
    fn quoted_argument_cannot_smuggle_no_store() {
        let policy = cache_policy_from_headers(&cc("private=\"no-store\", max-age=600")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 600_000 });
    }

    #[test]
    fn empty_max_age_value_is_typed_error() {
        let err = cache_policy_from_headers(&cc("max-age=")).unwrap_err();
        assert!(matches!(err, ParseError::InvalidMaxAge(s) if s.is_empty()));
    }

    #[test]
    fn max_age_overflow_beyond_u128_saturates() {
        let policy =
            cache_policy_from_headers(&cc(&format!("max-age={}", "9".repeat(45)))).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: u64::MAX });
    }

    #[test]
    fn swr_overflow_beyond_u128_saturates() {
        let policy = cache_policy_from_headers(&cc(&format!(
            "max-age=10, stale-while-revalidate={}",
            "9".repeat(45)
        )))
        .unwrap();
        assert_eq!(
            policy,
            CachePolicy::StaleWhileRevalidate {
                ttl_ms: 10_000,
                stale_ms: u64::MAX
            }
        );
    }

    #[test]
    fn tabs_around_equals_are_tolerated() {
        let policy = cache_policy_from_headers(&cc("max-age\t=\t60")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 60_000 });
    }

    #[test]
    fn invalid_value_is_truncated_in_error_text() {
        let err =
            cache_policy_from_headers(&cc(&format!("max-age={}", "x".repeat(600)))).unwrap_err();
        let text = err.to_string();
        assert!(
            text.ends_with("...[truncated]"),
            "unbounded error text: {} bytes",
            text.len()
        );
        assert!(
            text.len()
                <= "invalid max-age value: ".len() + ERROR_VALUE_MAX_BYTES + "...[truncated]".len()
        );
    }

    #[test]
    fn invalid_value_truncation_lands_on_char_boundary() {
        let raw = format!("{}{}", "x".repeat(511), "é".repeat(60));
        let err = parse_secs(false, &raw).unwrap_err();
        assert!(err.to_string().ends_with("...[truncated]"));
    }

    #[test]
    fn cache_meta_serde_roundtrip() {
        let meta = CacheMeta {
            etag: Some("\"abc\"".to_string()),
            last_modified: None,
            stored_at: std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            fresh_for: Duration::from_secs(60),
            stale_for: Duration::from_secs(120),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: CacheMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.etag, meta.etag);
        assert_eq!(back.last_modified, meta.last_modified);
        assert_eq!(back.fresh_for, meta.fresh_for);
        assert_eq!(back.stale_for, meta.stale_for);
        assert_eq!(back.stored_at, meta.stored_at);
    }
}

#[cfg(all(doctest, feature = "reqwest"))]
mod readme_doctests {
    #[doc = include_str!("../README.md")]
    struct Readme;
}
