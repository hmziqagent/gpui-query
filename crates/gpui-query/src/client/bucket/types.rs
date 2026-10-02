use gpui::WeakEntity;

use crate::core::RequestSequencer;

/// A `gc_time_ms` below this floor would evict every `Idle`/`Failure`
/// resource on every GC pass (unsigned ages always satisfy `age >= 0`).
pub(crate) const MIN_GC_TIME_MS: u64 = 1_000;

/// Entries per bucket before the oldest one is evicted; bounds memory when
/// components register unbounded unique keys.
pub(crate) const DEFAULT_MAX_ENTRIES: usize = 10_000;

/// Successful data outlives transient failures by this multiple of
/// `gc_time_ms`.
pub(crate) const SUCCESS_GC_MULTIPLIER: u32 = 2;

/// `updated_at` is the recency baseline when the resource has no completion
/// timestamp; the mirrors are refreshed wherever the bucket already reads the entity.
pub(crate) struct BucketEntry<R> {
    pub entity: WeakEntity<R>,
    pub sequencer: RequestSequencer,
    pub(crate) updated_at: u64,
    pub(crate) last_updated_ms: Option<u64>,
    pub(crate) loading: bool,
}
