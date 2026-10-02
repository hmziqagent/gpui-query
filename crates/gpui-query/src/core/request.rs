use serde::{Deserialize, Serialize};
use std::num::NonZero;

/// Scoped id: equality and ordering are lexicographic (scope, then sequence).
///
/// # Example
///
/// ```
/// use gpui_query::core::RequestId;
/// use std::num::NonZero;
///
/// let id = RequestId::scoped(NonZero::new(1).unwrap(), 42);
/// assert_eq!(id.scope_id(), NonZero::new(1).unwrap());
/// assert_eq!(id.value(), 42);
/// assert_eq!(id.label(), "1:42");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[must_use]
pub struct RequestId {
    scope_id: NonZero<u64>,
    sequence: u64,
}

impl RequestId {
    pub fn scoped(scope_id: NonZero<u64>, sequence: u64) -> Self {
        Self { scope_id, sequence }
    }

    pub fn value(self) -> u64 {
        self.sequence
    }

    pub fn scope_id(self) -> NonZero<u64> {
        self.scope_id
    }

    /// Allocates a `String`; `format!("{id}")` writes the same text without
    /// the heap allocation.
    pub fn label(self) -> String {
        self.to_string()
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.scope_id, self.sequence)
    }
}

/// Monotonic request id generator scoped to a single resource.
///
/// The sequence increments from 1; at `u64::MAX` the scope advances and the
/// sequence resets. If the scope itself overflows it wraps to 1, so a fresh
/// id could theoretically collide with a very old one still held by a
/// long-running future. Fallback mints via `next_fallback`
/// stay in a reserved top scope, disjoint from ids minted from 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestSequencer {
    pub(crate) scope_id: NonZero<u64>,
    pub(crate) next_request_id: u64,
}

impl Default for RequestSequencer {
    fn default() -> Self {
        Self::new()
    }
}

/// Id source for begin-request entry points: an external sequencer, or a
/// caller-provided id falling back to the resource's own sequencer.
pub(crate) enum MaybeRequestId<'a> {
    FromSequencer(&'a mut RequestSequencer),
    Provided(Option<RequestId>),
}

impl MaybeRequestId<'_> {
    pub(crate) fn next(&mut self, fallback: &mut RequestSequencer) -> RequestId {
        match self {
            Self::FromSequencer(sequencer) => sequencer.next_request(),
            Self::Provided(maybe_id) => maybe_id.unwrap_or_else(|| fallback.next_fallback()),
        }
    }
}

impl RequestSequencer {
    /// Reserved scope for resource-local fallback ids; `new()` sequencers
    /// start at 1, so the two id spaces stay disjoint.
    pub(crate) const RESERVED_FALLBACK_SCOPE: NonZero<u64> = NonZero::<u64>::MAX;

    pub fn new() -> Self {
        Self {
            scope_id: NonZero::<u64>::MIN,
            next_request_id: 1,
        }
    }

    /// Mints in the reserved fallback scope, re-scoping sequencers that start
    /// in the shared space (fresh or deserialized resources).
    pub(crate) fn next_fallback(&mut self) -> RequestId {
        if self.scope_id != Self::RESERVED_FALLBACK_SCOPE {
            self.scope_id = Self::RESERVED_FALLBACK_SCOPE;
            self.next_request_id = 1;
        }
        self.next_request()
    }

    pub fn next_request(&mut self) -> RequestId {
        let request_id = RequestId::scoped(self.scope_id, self.next_request_id);
        if self.next_request_id == u64::MAX {
            self.advance_scope();
        } else {
            self.next_request_id += 1;
        }
        request_id
    }

    pub fn advance_scope(&mut self) {
        self.scope_id = NonZero::new(self.scope_id.get().checked_add(1).unwrap_or(1))
            .unwrap_or(NonZero::<u64>::MIN);
        self.next_request_id = 1;
    }

    pub fn is_current_scope(&self, request_id: RequestId) -> bool {
        request_id.scope_id == self.scope_id
    }
}

/// Milliseconds since the UNIX epoch, driven by the application's clock
/// via [`QueryTimestamp::from_millis`]; core has no clock of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct QueryTimestamp(u64);

impl QueryTimestamp {
    pub fn from_millis(value: u64) -> Self {
        Self(value)
    }

    pub fn as_millis(self) -> u64 {
        self.0
    }

    pub(super) fn elapsed_since(self, earlier: Self) -> Option<u64> {
        self.0.checked_sub(earlier.0)
    }
}

impl From<u64> for QueryTimestamp {
    fn from(value: u64) -> Self {
        Self::from_millis(value)
    }
}

/// Single-use token moved into a `complete_*` call, enforcing
/// accept-then-complete at the type level.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub struct RequestGuard {
    request_id: RequestId,
}

impl RequestGuard {
    pub(super) fn new(request_id: RequestId) -> Self {
        Self { request_id }
    }

    pub fn request_id(&self) -> RequestId {
        self.request_id
    }

    pub fn into_request_id(self) -> RequestId {
        self.request_id
    }
}
