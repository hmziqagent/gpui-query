//! Async, value-carrying persistence for [`QueryClient`](super::QueryClient):
//! the [`Persister`] trait, the debounced [`QueryClient::persist_with`] driver,
//! [`hydrate`], and the typed serializer/deserializer registries.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, Subscription};
use serde_json::Value as JsonValue;
use thiserror::Error;

use crate::core::{CachePolicy, QueryKey};

use super::QueryClient;

/// Bumped when the serialized shape changes incompatibly; loaders reject
/// mismatches with [`PersistError::VersionMismatch`]. Version 2 made
/// [`QueryKey::to_path`] injective by escaping segments.
pub const PERSIST_VERSION: u32 = 2;

#[derive(Debug, Error)]
pub enum PersistError {
    #[error("persistence io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("persistence serialize error: {0}")]
    Serialize(#[from] serde_json::Error),
    /// Surfaced by persisters that decline to tolerate a parse failure; core
    /// itself never constructs this variant.
    #[error("persistence deserialize error: {0}")]
    Deserialize(String),
    #[error("persistence version mismatch: expected {expected}, found {found}")]
    VersionMismatch { expected: u32, found: u32 },
    #[error("persistence bad path: {0}")]
    BadPath(String),
    /// A required resource could not be acquired (e.g. a file lock).
    #[error("persistence permission denied: {0}")]
    Permission(String),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PersistedEntry {
    /// Opaque to core; typed round-trips go through the registries.
    pub value: JsonValue,
    /// Wall-clock ms since the UNIX epoch when the entry was cached.
    pub cached_at: u64,
    /// The cache policy in force when the entry was cached.
    pub cache_policy: CachePolicy,
    /// Opaque metadata captured from `Fetched::meta` (e.g. HTTP ETags),
    /// read by the `gpui-query-http` crate.
    pub meta: Option<JsonValue>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PersistSnapshot {
    /// Keyed by [`QueryKey`] path string so the snapshot is serializable.
    pub entries: HashMap<String, PersistedEntry>,
    /// See [`PERSIST_VERSION`].
    pub version: u32,
}

impl PersistSnapshot {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            version: PERSIST_VERSION,
        }
    }
}

/// Owned counterpart to [`QueryKeyFilter`](crate::core::QueryKeyFilter), so a
/// filter can be pinned inside [`PersistOptions`] and other long-lived values.
#[derive(Clone, Debug)]
pub enum PersistFilter {
    Exact(QueryKey),
    Prefix(QueryKey),
    All,
}

impl PersistFilter {
    pub fn matches(&self, key: &QueryKey) -> bool {
        match self {
            PersistFilter::Exact(target) => key == target,
            PersistFilter::Prefix(prefix) => key.starts_with(prefix),
            PersistFilter::All => true,
        }
    }
}

/// Defaults: every entry, max age 24 hours, 500 ms debounce.
#[derive(Clone, Debug)]
pub struct PersistOptions {
    pub filter: PersistFilter,
    /// Entries older than this are skipped at save time; zero disables the
    /// check.
    pub max_age: Duration,
    /// Coalesces bursts of [`CacheMutation`](super::CacheMutation) into one
    /// save per window; [`Duration::ZERO`] skips the delay entirely.
    pub debounce: Duration,
}

impl Default for PersistOptions {
    fn default() -> Self {
        Self {
            filter: PersistFilter::All,
            max_age: Duration::from_secs(24 * 60 * 60),
            debounce: Duration::from_millis(500),
        }
    }
}

/// `None` means the downcast failed; the caller skips the entry.
type SerializeFn = Box<dyn Fn(&dyn std::any::Any) -> Option<JsonValue> + Send + Sync>;

/// Keyed on `T` alone (matching the bucket lookup): registering the same `T`
/// under two error types overwrites, and the surviving closure applies to
/// every `(T, E)` bucket.
#[derive(Default)]
pub struct SerializerRegistry {
    serializers: HashMap<TypeId, SerializeFn>,
}

impl SerializerRegistry {
    pub fn register<T: 'static>(&mut self, f: fn(&T) -> JsonValue) {
        let wrap = move |any: &dyn std::any::Any| -> Option<JsonValue> {
            // Unreachable (lookup is by TypeId::of::<T>()); degrade instead of panicking.
            any.downcast_ref::<T>().map(f)
        };
        self.serializers.insert(TypeId::of::<T>(), Box::new(wrap));
    }

    pub(crate) fn get(&self, type_id: TypeId) -> Option<&SerializeFn> {
        self.serializers.get(&type_id)
    }

    pub fn contains(&self, type_id: TypeId) -> bool {
        self.serializers.contains_key(&type_id)
    }
}

type HydrateStep =
    Arc<dyn Fn(&mut QueryClient, &QueryKey, &JsonValue, &mut App) -> bool + Send + Sync>;

/// Hydrate steps consumed by [`hydrate`]; a step returns `true` when it
/// decoded and primed the value.
#[derive(Default)]
pub struct DeserializerRegistry {
    steps: Vec<HydrateStep>,
}

impl DeserializerRegistry {
    /// `None` from `deserialize` skips the entry; `Some(t)` primes the value
    /// via `set_query_data::<T, E>`.
    pub fn register<T, E>(&mut self, deserialize: fn(&JsonValue) -> Option<T>)
    where
        T: Clone + Send + Sync + 'static,
        E: Clone + Send + Sync + 'static,
    {
        let step = move |client: &mut QueryClient,
                         key: &QueryKey,
                         value: &JsonValue,
                         cx: &mut App|
              -> bool {
            let Some(t) = deserialize(value) else {
                return false;
            };
            client.set_query_data::<T, E>(key.clone(), t, cx);
            true
        };
        self.steps.push(Arc::new(step));
    }

    fn iter(&self) -> impl Iterator<Item = &HydrateStep> {
        self.steps.iter()
    }
}

/// Async persistence backend for [`QueryClient::persist_with`]. Not
/// object-safe (methods return `impl Future`): the driver monomorphizes over
/// the concrete `P`, and saves run on GPUI's background executor. See the
/// `gpui-query-persist` satellite crate for a reference disk implementation.
pub trait Persister: Send + Sync + 'static {
    /// Implementations should tolerate a missing or corrupt store by yielding
    /// an empty snapshot.
    fn load(&self) -> impl Future<Output = Result<PersistSnapshot, PersistError>> + Send;

    /// Replaces any previously stored data.
    fn save(
        &self,
        snapshot: &PersistSnapshot,
    ) -> impl Future<Output = Result<(), PersistError>> + Send;
}

/// Drop guard for [`QueryClient::persist_with`]: dropping it stops new saves,
/// but an already-armed task still collects and completes its final save.
pub struct PersistHandle {
    _subscription: Option<Subscription>,
}

impl PersistHandle {
    /// Observes nothing; for tests and no-op setups.
    pub fn empty() -> Self {
        Self {
            _subscription: None,
        }
    }
}

impl QueryClient {
    /// Only `Success` resources whose `T` has a registered serializer are
    /// collected; unregistered types are skipped.
    pub fn register_serializer<T, E>(&mut self, f: fn(&T) -> JsonValue)
    where
        T: Clone + Send + Sync + 'static,
        E: Clone + Send + Sync + 'static,
    {
        let registry = self
            .serializers
            .get_or_insert_with(SerializerRegistry::default);
        registry.register::<T>(f);
    }

    /// [`hydrate`] offers every on-disk entry to every registered
    /// deserializer (there is no type discriminator on [`PersistedEntry`]):
    /// one that accepts a JSON shape that is not its own `T` primes a foreign
    /// value into a bucket it does not belong to.
    pub fn register_deserializer<T, E>(&mut self, deserialize: fn(&JsonValue) -> Option<T>)
    where
        T: Clone + Send + Sync + 'static,
        E: Clone + Send + Sync + 'static,
    {
        let registry = self
            .deserializers
            .get_or_insert_with(DeserializerRegistry::default);
        registry.register::<T, E>(deserialize);
    }

    /// Only `Success` resources with a registered serializer are included.
    pub fn collect_persist_snapshot(
        &self,
        filter: &PersistFilter,
        max_age: Duration,
        cx: &App,
    ) -> PersistSnapshot {
        let flushed = HashMap::new();
        let mut out = self.collect_persist_delta(filter, max_age, &flushed, cx);
        let mut snapshot = PersistSnapshot::new();
        snapshot.entries = out
            .fresh
            .drain(..)
            .map(|collected| (collected.path, collected.entry))
            .collect();
        snapshot
    }

    /// Walks the query buckets once, serializing only entries whose data
    /// epoch differs from `flushed` (their last-flushed value). Live entries
    /// already flushed come back as reused paths; the caller merges fresh
    /// and reused entries over its accumulated store.
    pub(crate) fn collect_persist_delta(
        &self,
        filter: &PersistFilter,
        max_age: Duration,
        flushed: &HashMap<String, (gpui::EntityId, u64)>,
        cx: &App,
    ) -> crate::client::bucket::shared::PersistCollectOut {
        let Some(ref registry) = self.serializers else {
            return crate::client::bucket::shared::PersistCollectOut::default();
        };
        let now_ms = crate::client::time::current_time_ms();
        let max_age_ms = max_age.as_millis() as u64;
        let collect = crate::client::bucket::shared::PersistCollect::new(
            registry, filter, now_ms, max_age_ms, flushed,
        );

        let mut out = crate::client::bucket::shared::PersistCollectOut::default();
        for bucket in self.buckets.values() {
            bucket.collect_persistable_into(cx, &collect, &mut out);
        }
        for bucket in self.infinite_buckets.values() {
            bucket.collect_persistable_into(cx, &collect, &mut out);
        }

        if let Some(meta_map) = &self.persisted_meta {
            for collected in &mut out.fresh {
                if let Some(m) = meta_map.get(&collected.key) {
                    collected.entry.meta = Some(m.clone());
                }
            }
        }
        out
    }

    /// Debounced [`Persister`] driver on the [`CacheMutation`](super::CacheMutation)
    /// dirty signal: collection happens at drain time, so a burst of bumps
    /// coalesces into one save of the latest state.
    ///
    /// Saves carry the full accumulated store, but only dirty entries (data
    /// epoch changed since the last flush) are re-serialized; entries
    /// removed from the cache are pruned. A flush with no dirty entries and
    /// no prunes does not save at all. Saves run one at a time: a flush
    /// landing while a save is in flight queues a follow-up flush, so
    /// snapshots reach the persister in flush order.
    pub fn persist_with<P: Persister>(
        &self,
        persister: P,
        opts: PersistOptions,
        cx: &mut App,
    ) -> PersistHandle {
        let persister: Arc<P> = Arc::new(persister);
        let debounce = opts.debounce;
        let bg = cx.background_executor().clone();
        let armed = Arc::new(AtomicBool::new(false));
        let flush_state = Arc::new(Mutex::new(PersistFlushState {
            flushed: HashMap::new(),
            store: Arc::new(PersistSnapshot::new()),
            save_in_flight: false,
            save_queued: false,
        }));

        let _ = cx.default_global::<super::CacheMutation>();

        let subscription = {
            let persister = persister.clone();
            let armed = armed.clone();
            let filter = opts.filter;
            let max_age = opts.max_age;
            let flush_state = flush_state.clone();
            cx.observe_global::<super::CacheMutation>(move |cx| {
                if armed.swap(true, Ordering::AcqRel) {
                    return;
                }
                let persister = persister.clone();
                let filter = filter.clone();
                let armed = armed.clone();
                let flush_state = flush_state.clone();
                let bg = bg.clone();
                cx.spawn(async move |cx| {
                    if !debounce.is_zero() {
                        bg.timer(debounce).await;
                    }
                    // Disarm before collecting: a bump landing now arms a fresh task.
                    armed.store(false, Ordering::Release);
                    loop {
                        let delta = cx.update_global::<QueryClient, _>(|client, cx| {
                            let Ok(state) = flush_state.lock() else {
                                return None;
                            };
                            Some(client.collect_persist_delta(&filter, max_age, &state.flushed, cx))
                        });
                        let Some(delta) = delta.ok().flatten() else {
                            return;
                        };
                        // Collect on the main thread (entity reads), save on background (IO).
                        let out = {
                            let Ok(mut state) = flush_state.lock() else {
                                return;
                            };
                            let live: HashSet<String> = delta
                                .fresh
                                .iter()
                                .map(|c| c.path.clone())
                                .chain(delta.reused.iter().cloned())
                                .collect();
                            let prune = state.store.entries.keys().any(|path| !live.contains(path));
                            if delta.fresh.is_empty() && !prune {
                                return;
                            }
                            if state.save_in_flight {
                                // The in-flight owner re-collects after its save, keeping saves ordered.
                                state.save_queued = true;
                                return;
                            }
                            let fresh_epochs: Vec<(String, (gpui::EntityId, u64))> = delta
                                .fresh
                                .iter()
                                .map(|c| (c.path.clone(), (c.entity_id, c.epoch)))
                                .collect();
                            {
                                let snapshot = Arc::make_mut(&mut state.store);
                                if prune {
                                    snapshot.entries.retain(|path, _| live.contains(path));
                                }
                                for collected in delta.fresh {
                                    snapshot.entries.insert(collected.path, collected.entry);
                                }
                            }
                            if prune {
                                state.flushed.retain(|path, _| live.contains(path));
                            }
                            state.flushed.extend(fresh_epochs);
                            state.save_in_flight = true;
                            state.store.clone()
                        };
                        let persister = persister.clone();
                        let result = bg.spawn(async move { persister.save(&out).await }).await;
                        let Ok(mut state) = flush_state.lock() else {
                            return;
                        };
                        state.save_in_flight = false;
                        if let Err(err) = result {
                            eprintln!("persist_with: save failed: {}", save_failure_log_text(&err));
                        }
                        if !state.save_queued {
                            return;
                        }
                        state.save_queued = false;
                    }
                })
                .detach();
            })
        };

        PersistHandle {
            _subscription: Some(subscription),
        }
    }
}

/// Per-driver flush state: the owning entity id and data epoch each path was
/// last flushed at, plus the full store as of the last save. Main-thread
/// only; the mutex guards the save handoff and the in-flight/queued pair
/// that keeps saves ordered.
struct PersistFlushState {
    flushed: HashMap<String, (gpui::EntityId, u64)>,
    store: Arc<PersistSnapshot>,
    save_in_flight: bool,
    save_queued: bool,
}

/// Persister errors can embed payload or path detail (custom `Deserialize`
/// strings, `Permission` text), so log output passes the shared redactor.
pub(crate) fn save_failure_log_text(err: &PersistError) -> String {
    crate::core::error::sanitize::sanitize_message(&err.to_string())
}

/// Persists nothing; loads an empty snapshot.
pub struct NoopPersister;

impl Persister for NoopPersister {
    async fn load(&self) -> Result<PersistSnapshot, PersistError> {
        Ok(PersistSnapshot::new())
    }

    async fn save(&self, _snapshot: &PersistSnapshot) -> Result<(), PersistError> {
        Ok(())
    }
}

/// Load a snapshot and re-prime the live cache with it: the value-carrying
/// counterpart to the metadata-only
/// [`QueryClient::hydrate`](super::QueryClient::hydrate). Stored `to_path()`
/// keys are decoded with `QueryKey::from_path` so `Exact`/`Prefix` filters
/// match live multi-segment keys. Returns the post-filter snapshot so
/// callers can inspect entries or prime types with no registered
/// deserializer. The persister's output is trusted beyond the version
/// check: one reading untrusted storage must validate payloads itself.
pub async fn hydrate<P: Persister>(
    client: &mut QueryClient,
    persister: &P,
    filter: &PersistFilter,
    max_age: Duration,
    cx: &mut App,
) -> Result<PersistSnapshot, PersistError> {
    let snapshot = persister.load().await?;
    // Checked here too so an in-memory persister cannot bypass the version gate.
    if snapshot.version != PERSIST_VERSION {
        return Err(PersistError::VersionMismatch {
            expected: PERSIST_VERSION,
            found: snapshot.version,
        });
    }
    let now_ms = crate::client::time::current_time_ms();
    let max_age_ms = max_age.as_millis() as u64;

    let Some(deserializers) = client.deserializers.as_ref() else {
        return Ok(snapshot);
    };

    let steps: Vec<HydrateStep> = deserializers.iter().cloned().collect();

    for (key_path, entry) in &snapshot.entries {
        let key = QueryKey::from_path(key_path);
        if !filter.matches(&key) {
            continue;
        }
        if max_age_ms > 0 && now_ms.saturating_sub(entry.cached_at) > max_age_ms {
            continue;
        }
        for step in &steps {
            step(client, &key, &entry.value, cx);
        }
    }

    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_failure_log_text_redacts_hostile_detail_and_keeps_benign() {
        let hostile =
            PersistError::Permission("cache write denied for token= supersecretvalue".to_string());
        let logged = save_failure_log_text(&hostile);
        assert!(logged.contains("[REDACTED_TOKEN]"), "{logged}");
        assert!(!logged.contains("supersecretvalue"));

        let path = PersistError::BadPath("no cache dir for /home/alice/app".to_string());
        let logged = save_failure_log_text(&path);
        assert!(logged.contains("[REDACTED_PATH]"), "{logged}");
        assert!(!logged.contains("/home/alice/app"));

        let benign = PersistError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "cache file missing",
        ));
        assert_eq!(
            save_failure_log_text(&benign),
            benign.to_string(),
            "benign io diagnostics must pass through unredacted"
        );
    }
}
