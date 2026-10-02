//! Cache and request policies for query resources.

use serde::{Deserialize, Serialize};

use super::{QueryStatus, RequestId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CachePolicy {
    NoCache,
    /// `ttl_ms = 0` behaves like `NoCache` (data is only "fresh" at the
    /// instant it is stored); only `debug_assert`ed, not validated in release.
    Ttl {
        ttl_ms: u64,
    },
    /// Fresh within `ttl_ms`; between `ttl_ms` and `ttl_ms + stale_ms` the
    /// stale data is served and a background revalidation is triggered; past
    /// that, a normal fetch runs. Zero values degenerate as in [`Ttl`](Self::Ttl);
    /// only `debug_assert`ed, not validated in release.
    StaleWhileRevalidate {
        ttl_ms: u64,
        stale_ms: u64,
    },
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self::Ttl { ttl_ms: 60_000 }
    }
}

impl CachePolicy {
    /// Allocates a `String`; `format!("{policy}")` writes the same text
    /// without the heap allocation.
    pub fn label(self) -> String {
        self.to_string()
    }

    pub fn can_short_circuit(self) -> bool {
        matches!(self, Self::Ttl { .. } | Self::StaleWhileRevalidate { .. })
    }

    pub fn can_serve_stale(self) -> bool {
        matches!(self, Self::StaleWhileRevalidate { .. })
    }

    pub fn ttl_ms(self) -> Option<u64> {
        match self {
            Self::NoCache => None,
            Self::Ttl { ttl_ms } | Self::StaleWhileRevalidate { ttl_ms, .. } => Some(ttl_ms),
        }
    }

    pub fn stale_ms(self) -> Option<u64> {
        match self {
            Self::StaleWhileRevalidate { stale_ms, .. } => Some(stale_ms),
            _ => None,
        }
    }

    /// TTL + stale, saturating on overflow: the maximum age at which data can
    /// still be served under this policy.
    pub fn total_valid_ms(self) -> Option<u64> {
        match self {
            Self::NoCache => None,
            Self::Ttl { ttl_ms } => {
                debug_assert!(
                    ttl_ms > 0,
                    "CachePolicy::Ttl with ttl_ms=0 behaves like NoCache"
                );
                Some(ttl_ms)
            }
            Self::StaleWhileRevalidate { ttl_ms, stale_ms } => {
                debug_assert!(
                    ttl_ms > 0,
                    "CachePolicy::StaleWhileRevalidate with ttl_ms=0 behaves like NoCache"
                );
                debug_assert!(
                    stale_ms > 0,
                    "CachePolicy::StaleWhileRevalidate with stale_ms=0 degenerates to Ttl-only behavior"
                );
                Some(ttl_ms.saturating_add(stale_ms))
            }
        }
    }

    pub fn is_fresh(self, age_ms: u64) -> bool {
        self.ttl_ms().is_some_and(|ttl| age_ms <= ttl)
    }

    pub fn is_stale_but_serveable(self, age_ms: u64) -> bool {
        match self {
            Self::StaleWhileRevalidate { ttl_ms, stale_ms } => {
                let total = ttl_ms.saturating_add(stale_ms);
                age_ms > ttl_ms && age_ms <= total
            }
            _ => false,
        }
    }

    pub fn is_expired(self, age_ms: u64) -> bool {
        self.total_valid_ms()
            .map(|total| age_ms > total)
            .unwrap_or(true)
    }
}

impl std::fmt::Display for CachePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCache => write!(f, "No cache"),
            Self::Ttl { ttl_ms } => {
                write!(f, "Cache TTL ")?;
                write_duration(f, *ttl_ms)
            }
            Self::StaleWhileRevalidate { ttl_ms, stale_ms } => {
                write!(f, "Stale-while-revalidate TTL ")?;
                write_duration(f, *ttl_ms)?;
                write!(f, " stale ")?;
                write_duration(f, *stale_ms)
            }
        }
    }
}

/// Seconds for `>= 1000ms`, milliseconds otherwise, so sub-second values do
/// not collapse to "0s" through integer division.
fn write_duration(f: &mut std::fmt::Formatter<'_>, ms: u64) -> std::fmt::Result {
    if ms >= 1_000 {
        write!(f, "{}s", ms / 1_000)
    } else {
        write!(f, "{ms}ms")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestPolicy {
    #[default]
    LatestWins,
    IgnoreWhileLoading,
}

impl RequestPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::LatestWins => "Latest wins",
            Self::IgnoreWhileLoading => "Ignore while loading",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueryFetchMode {
    #[default]
    Normal,
    Force,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum QueryBeginResult {
    Started {
        request_id: RequestId,
        status: QueryStatus,
        replaced_request_id: Option<RequestId>,
    },
    CacheHit,
    /// Serve the stale data immediately, run a background fetch with
    /// `request_id`, then complete normally.
    StaleCacheHit {
        request_id: RequestId,
        status: QueryStatus,
        replaced_request_id: Option<RequestId>,
    },
    IgnoredWhileLoading {
        active_request_id: RequestId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_zero_label_uses_ms() {
        let policy = CachePolicy::Ttl { ttl_ms: 0 };
        assert_eq!(policy.label(), "Cache TTL 0ms");
    }

    #[test]
    fn swr_zero_values_label_uses_ms() {
        let policy = CachePolicy::StaleWhileRevalidate {
            ttl_ms: 0,
            stale_ms: 0,
        };
        assert_eq!(policy.label(), "Stale-while-revalidate TTL 0ms stale 0ms");
    }

    #[test]
    fn total_valid_ms_saturates_on_overflow() {
        let policy = CachePolicy::StaleWhileRevalidate {
            ttl_ms: u64::MAX,
            stale_ms: u64::MAX,
        };
        assert_eq!(policy.total_valid_ms(), Some(u64::MAX));
    }
}
