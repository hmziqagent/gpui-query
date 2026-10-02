//! Type-erased bucket traits: let `QueryClient` store heterogeneous buckets
//! in `AHashMap<TypeId, Box<dyn Erased*>>` maps.

use crate::client::devtools::{MutationDiagnostic, QueryDiagnostic};
use crate::core::QueryKeyFilter;
#[cfg(feature = "persist")]
use crate::core::{MutationStatus, QueryStatus};

/// Erased surface behind both query maps in `QueryClient`; `TypeId` keys
/// keep the downcast to the concrete bucket sound.
pub(crate) trait ErasedBucket {
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    fn gc(&mut self, now_ms: u64, gc_time_ms: u64, cx: &gpui::App);
    fn count(&self) -> usize;
    fn invalidate_matching(&mut self, filter: &QueryKeyFilter, cx: &mut gpui::App);
    fn reset_matching(&mut self, filter: &QueryKeyFilter, cx: &mut gpui::App);
    fn remove_matching(&mut self, filter: &QueryKeyFilter);
    fn cancel_matching(&mut self, filter: &QueryKeyFilter, cx: &mut gpui::App);
    /// Pushes into the caller's Vec so `QueryClient::diagnostics` pre-sizes
    /// one destination instead of allocating per bucket.
    fn collect_diagnostics_into(&self, now_ms: u64, cx: &gpui::App, out: &mut Vec<QueryDiagnostic>);
    /// Key/status pairs without the per-entry allocations of full
    /// diagnostics; used by `dehydrate`.
    #[cfg(feature = "persist")]
    fn collect_key_status_into(&self, cx: &gpui::App, out: &mut Vec<(String, QueryStatus)>);
    /// Entries whose `T` has no registered serializer are skipped; filter,
    /// max-age, and the driver's last-flushed epochs are checked before
    /// serializing so skipped entries cost nothing.
    #[cfg(feature = "persist")]
    fn collect_persistable_into(
        &self,
        cx: &gpui::App,
        collect: &crate::client::bucket::shared::PersistCollect<'_>,
        out: &mut crate::client::bucket::shared::PersistCollectOut,
    );
    /// Prunes the persisted-meta map of keys whose entries were evicted.
    #[cfg(feature = "persist")]
    fn contains_key(&self, key: &crate::core::QueryKey) -> bool;
}

pub(crate) trait ErasedMutationBucket {
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    fn gc(&mut self, now_ms: u64, gc_time_ms: u64, cx: &gpui::App);
    fn count(&self) -> usize;
    fn collect_diagnostics_into(&self, cx: &gpui::App, out: &mut Vec<MutationDiagnostic>);
    /// `key` is `None` for keyless mutations.
    #[cfg(feature = "persist")]
    fn collect_key_status_into(
        &self,
        cx: &gpui::App,
        out: &mut Vec<(Option<String>, MutationStatus)>,
    );
}

/// Legacy metadata-only persistence: entries serialize as JSON strings,
/// avoiding generic bounds. The async value-carrying surface is
/// [`Persister`](crate::client::Persister) plus
/// [`persist_with`](crate::client::QueryClient::persist_with).
///
/// # Example
///
/// ```
/// use std::path::PathBuf;
/// use gpui_query::client::{QueryPersister, DehydratedEntry};
///
/// struct FilePersister { path: PathBuf }
///
/// impl QueryPersister for FilePersister {
///     fn load(&self) -> Vec<DehydratedEntry> { Vec::new() }
///     fn save(&self, _entries: Vec<DehydratedEntry>) {}
/// }
/// ```
#[cfg(feature = "persist")]
pub trait QueryPersister: Send + Sync {
    fn load(&self) -> Vec<crate::client::devtools::DehydratedEntry>;

    /// Replaces any previously stored data.
    fn save(&self, entries: Vec<crate::client::devtools::DehydratedEntry>);
}
