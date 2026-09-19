//! Reference disk-based persistence adapter for [`gpui_query`].
//!
//! Provides [`FilePersister`], a [`Persister`]
//! implementation that atomically writes a [`PersistSnapshot`] to disk and
//! tolerantly loads it back, plus a [`NoopPersister`] for tests/disabled modes.
//!
//! # Atomic write
//!
//! Each save serializes the snapshot to a sibling `NamedTempFile` (via
//! [`tempfile`]), fsyncs it (issuing `F_FULLFSYNC` on macOS for true durability,
//! plain `fsync` elsewhere), then renames it over the target (`rename(2)` on
//! POSIX, `MoveFileEx` semantics on Windows via tempfile's `.persist()`). On
//! POSIX the parent directory is fsynced after the replace so the rename is
//! durable across power loss.
//!
//! # Tolerant load
//!
//! - Missing file → empty snapshot.
//! - Corrupt / unparseable file → logged + empty snapshot (no panic).
//! - Version mismatch → [`PersistError::VersionMismatch`] (typed, so callers
//!   can distinguish "corrupt" from "wrong format").
//!
//! # Concurrency
//!
//! Writes are serialized via a `std::sync::Mutex` so concurrent `save` calls
//! from the GPUI background executor never interleave temp-file lifecycles.
//! Reads take the same lock briefly. No `tokio` is required: the persister
//! runs its async methods on GPUI's `background_executor`.
//!
//! # Blocking I/O note
//!
//! `FilePersister` performs synchronous `std::fs` I/O inside its async `load`
//! and `save` bodies and is intended to run on GPUI's `background_executor`,
//! which is a dedicated blocking-friendly thread pool. Callers running on a
//! `tokio` multi-thread runtime that need non-blocking semantics should wrap
//! `load`/`save` in `spawn_blocking` (e.g. via a `tokio::task::spawn_blocking`
//! → bridge) to avoid stalling executor threads.
//!
//! # Windows `ERROR_ACCESS_DENIED` (retryable)
//!
//! On Windows the atomic replace can fail with `ERROR_ACCESS_DENIED` when an
//! antivirus scanner or concurrent reader holds the destination. This is
//! surfaced as [`PersistError::Permission`] (rather than flattened into an IO
//! error), preserving the retryable signal so callers can back off and retry
//! the save. All other persist failures are returned as
//! [`PersistError::Io`] carrying the original `std::io::Error` (kind + source
//! chain intact) so no diagnostic detail is lost.

#![deny(missing_docs)]

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gpui_query::client::{
    PERSIST_VERSION, PersistError, PersistSnapshot, PersistedEntry, Persister,
};
use gpui_query::core::CachePolicy;

/// On-disk serialization format for [`FilePersister`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistFormat {
    /// Human-readable JSON (`serde_json`). Default; easy to inspect/debug.
    Json,
    /// Compact binary (`bincode`). Smaller and faster; not human-readable.
    Bincode,
}

/// Atomic, durable disk-backed [`Persister`].
///
/// Writes go to a sibling temp file (via [`tempfile::NamedTempFile`]), which is
/// fsynced and then renamed over the target so a crash mid-write never leaves a
/// truncated/corrupt cache file. See the [crate-level docs](crate) for the full
/// durability story.
pub struct FilePersister {
    path: PathBuf,
    format: PersistFormat,
    write_lock: Mutex<()>,
}

impl FilePersister {
    /// Construct a persister writing to `path` in the given `format`.
    pub fn new(path: impl Into<PathBuf>, format: PersistFormat) -> Self {
        Self {
            path: path.into(),
            format,
            write_lock: Mutex::new(()),
        }
    }

    /// Construct a JSON persister at `path`.
    pub fn json(path: impl Into<PathBuf>) -> Self {
        Self::new(path, PersistFormat::Json)
    }

    /// Construct a bincode persister at `path`.
    pub fn bincode(path: impl Into<PathBuf>) -> Self {
        Self::new(path, PersistFormat::Bincode)
    }

    /// Construct a persister rooted at the OS cache directory (`dirs::cache_dir`)
    /// joined with `app_name`, storing `cache` (JSON format).
    ///
    /// Returns [`PersistError::BadPath`] when the OS reports no cache dir
    /// (e.g. the platform does not define one). This matches the Open Question 8
    /// decision in the design doc: `cache_dir` (regenerable offline cache)
    /// rather than Roaming, so a cold start after launch reconstructs the cache
    /// without syncing stale state.
    pub fn in_cache_dir(app_name: impl AsRef<str>) -> Result<Self, PersistError> {
        let app_name = app_name.as_ref();
        let dir = dirs::cache_dir().ok_or_else(|| {
            PersistError::BadPath(format!("no OS cache dir available for app {app_name:?}"))
        })?;
        let path = dir.join(app_name).join("gpui-query-cache.json");
        Ok(Self::json(path))
    }

    /// The on-disk path this persister writes to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Serialize + atomically write `snapshot` to disk.
    fn write_atomic(&self, snapshot: &PersistSnapshot) -> Result<(), PersistError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| PersistError::Permission("write lock poisoned".to_string()))?;

        // Ensure the parent directory exists (best effort; a missing parent is
        // an error we propagate).
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }

        let bytes: Vec<u8> = match self.format {
            PersistFormat::Json => serde_json::to_vec(snapshot)?,
            // bincode cannot (de)serialize `serde_json::Value` directly (its
            // Deserialize impl uses `deserialize_any`, which bincode's
            // non-self-describing format can't drive). So for the bincode
            // format we JSON-encode each entry's `value` to a String inside a
            // bincode-safe adapter, then bincode the adapter.
            PersistFormat::Bincode => {
                let adapter = BincodeSnapshot::from_snapshot(snapshot)?;
                bincode::serialize(&adapter).map_err(|e| {
                    use serde::ser::Error as _;
                    PersistError::Serialize(serde_json::Error::custom(e.to_string()))
                })?
            }
        };

        // Write to a sibling NamedTempFile, fsync, then rename over the target.
        // `tempfile::NamedTempFile::persist` performs the atomic replace.
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        let mut tmp = tempfile::Builder::new()
            .prefix(
                self.path
                    .file_name()
                    .map(Path::new)
                    .unwrap_or_else(|| Path::new("cache")),
            )
            .tempfile_in(parent)?;
        tmp.write_all(&bytes)?;
        tmp.as_file().sync_all().map_err(fullfsync_err)?;
        // Promote F_FULLFSYNC on macOS for true durability.
        #[cfg(target_os = "macos")]
        try_fullfsync(tmp.as_file());
        // tempfile's persist failure exposes the underlying io::Error via its
        // `.error` field. We preserve that error's real ErrorKind/source chain
        // (rather than flattening to a string) so callers can match on it.
        //
        // On Windows, an AV scanner or concurrent reader holding the destination
        // surfaces `ERROR_ACCESS_DENIED` (5) from `MoveFileEx`; the std
        // ErrorKind for that is `PermissionDenied`. Per the design doc, that
        // case is *retryable* and is mapped to `PersistError::Permission` so a
        // caller can back off and retry the save. Everything else is preserved
        // as `PersistError::Io` with the original error (kind + source).
        tmp.persist(&self.path).map_err(|persist_err| {
            let io_err = persist_err.error;
            let kind = io_err.kind();
            let is_access_denied = kind == std::io::ErrorKind::PermissionDenied
                || is_windows_access_denied(io_err.raw_os_error());
            if is_access_denied {
                PersistError::Permission(format!(
                    "atomic persist of cache file was denied (retryable): {io_err}"
                ))
            } else {
                PersistError::Io(io_err)
            }
        })?;

        // On POSIX, fsync the parent directory so the rename is durable.
        #[cfg(unix)]
        fsync_parent(parent);

        Ok(())
    }

    /// Tolerantly read + deserialize the snapshot from disk.
    fn read_tolerant(&self) -> Result<PersistSnapshot, PersistError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| PersistError::Permission("write lock poisoned".to_string()))?;

        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Missing file → empty snapshot.
                return Ok(empty_snapshot());
            }
            Err(e) => return Err(e.into()),
        };

        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        let snapshot: PersistSnapshot = match self.format {
            PersistFormat::Json => match serde_json::from_slice(&buf) {
                Ok(s) => s,
                Err(e) => {
                    // Corrupt JSON → empty snapshot + log (tolerant load).
                    log_or_eprint(&format!(
                        "FilePersister: corrupt JSON cache at {}: {e}; treating as empty",
                        self.path.display()
                    ));
                    return Ok(empty_snapshot());
                }
            },
            PersistFormat::Bincode => match bincode::deserialize::<BincodeSnapshot>(&buf) {
                Ok(adapter) => match adapter.into_snapshot() {
                    Ok(s) => s,
                    Err(e) => {
                        log_or_eprint(&format!(
                            "FilePersister: corrupt bincode cache at {}: {e}; treating as empty",
                            self.path.display()
                        ));
                        return Ok(empty_snapshot());
                    }
                },
                Err(e) => {
                    log_or_eprint(&format!(
                        "FilePersister: corrupt bincode cache at {}: {e}; treating as empty",
                        self.path.display()
                    ));
                    return Ok(empty_snapshot());
                }
            },
        };

        if snapshot.version != PERSIST_VERSION {
            return Err(PersistError::VersionMismatch {
                expected: PERSIST_VERSION,
                found: snapshot.version,
            });
        }
        Ok(snapshot)
    }
}

impl Persister for FilePersister {
    async fn load(&self) -> Result<PersistSnapshot, PersistError> {
        self.read_tolerant()
    }

    async fn save(&self, snapshot: &PersistSnapshot) -> Result<(), PersistError> {
        self.write_atomic(snapshot)
    }
}

/// A [`Persister`] that persists nothing and loads an empty snapshot.
///
/// Re-exported from `gpui_query::client` for the no-op case; this is the local
/// crate copy kept so consumers of `gpui-query-persist` have a one-stop import.
pub use gpui_query::client::NoopPersister;

// ── helpers ─────────────────────────────────────────────────────────────

/// Bincode-safe adapter for [`PersistSnapshot`].
///
/// `serde_json::Value`'s `Deserialize` impl uses `deserialize_any`, which
/// bincode's non-self-describing format cannot drive. This adapter stores each
/// entry's `value` as a JSON `String` (the bytes round-trip through
/// `serde_json`), so bincode — which handles `String`, `u64`, `Option`, and
/// `HashMap` natively — can serialize the snapshot. The conversion is
/// lossless.
#[derive(serde::Serialize, serde::Deserialize)]
struct BincodeSnapshot {
    entries: HashMap<String, BincodeEntry>,
    version: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct BincodeEntry {
    /// The entry's value, JSON-encoded to a String so bincode can carry it.
    value_json: String,
    cached_at: u64,
    cache_policy: CachePolicy,
    meta_json: Option<String>,
}

impl BincodeSnapshot {
    fn from_snapshot(s: &PersistSnapshot) -> Result<Self, PersistError> {
        let mut entries = HashMap::with_capacity(s.entries.len());
        for (k, e) in &s.entries {
            entries.insert(
                k.clone(),
                BincodeEntry {
                    value_json: serde_json::to_string(&e.value)?,
                    cached_at: e.cached_at,
                    cache_policy: e.cache_policy,
                    meta_json: match &e.meta {
                        Some(m) => Some(serde_json::to_string(m)?),
                        None => None,
                    },
                },
            );
        }
        Ok(Self {
            entries,
            version: s.version,
        })
    }

    fn into_snapshot(self) -> Result<PersistSnapshot, PersistError> {
        let mut entries = HashMap::with_capacity(self.entries.len());
        for (k, e) in self.entries {
            let value: serde_json::Value = serde_json::from_str(&e.value_json)?;
            let meta = match e.meta_json {
                Some(m) => Some(serde_json::from_str(&m)?),
                None => None,
            };
            entries.insert(
                k,
                PersistedEntry {
                    value,
                    cached_at: e.cached_at,
                    cache_policy: e.cache_policy,
                    meta,
                },
            );
        }
        Ok(PersistSnapshot {
            entries,
            version: self.version,
        })
    }
}

fn empty_snapshot() -> PersistSnapshot {
    PersistSnapshot {
        entries: HashMap::new(),
        version: PERSIST_VERSION,
    }
}

/// Log via `eprintln!` (no `log` dep). Tolerant-load warnings land here.
fn log_or_eprint(msg: &str) {
    eprintln!("{msg}");
}

/// Map a failed `sync_all` to a `PersistError`, capturing the platform detail.
///
/// On macOS `sync_all` already issues `fsync`; `try_fullfsync` then attempts the
/// stronger `F_FULLFSYNC`. A failure here is propagated as an IO error.
fn fullfsync_err(e: std::io::Error) -> PersistError {
    PersistError::Io(e)
}

/// Returns `true` if the given raw OS error is Windows `ERROR_ACCESS_DENIED`.
///
/// On Windows, antivirus scanners and concurrent readers can cause
/// `MoveFileEx` to fail with `ERROR_ACCESS_DENIED` (5) during the atomic
/// replace (rust-lang/rust#123985). Such failures are transient and retryable.
/// `std::io::ErrorKind::PermissionDenied` already maps this on Windows, but we
/// also check the raw code defensively (e.g. for errors constructed via
/// `from_raw_os_error` whose kind may not be normalized uniformly).
#[cfg(windows)]
const ERROR_ACCESS_DENIED: i32 = 5;
fn is_windows_access_denied(raw: Option<i32>) -> bool {
    #[cfg(windows)]
    {
        raw == Some(ERROR_ACCESS_DENIED)
    }
    #[cfg(not(windows))]
    {
        let _ = raw;
        false
    }
}

/// Issue `F_FULLFSYNC` on macOS for true durability (flushes the drive cache).
///
/// `fsync` only flushes the kernel buffer cache; `F_FULLFSYNC` asks the drive to
/// flush its own write cache. Best-effort: a failure is logged but does not
/// fail the save (the prior `sync_all` already provides kernel-level durability).
#[cfg(target_os = "macos")]
fn try_fullfsync(file: &File) {
    // F_FULLFSYNC = 0x00008027 (fcntl.h on Darwin). We declare the extern
    // rather than taking a libc dependency.
    unsafe extern "C" {
        fn fcntl(fd: std::os::fd::RawFd, cmd: std::ffi::c_int, ...) -> std::ffi::c_int;
    }
    const F_FULLFSYNC: std::ffi::c_int = 0x00008027;
    use std::os::fd::AsRawFd;
    let fd = file.as_raw_fd();
    // SAFETY: `F_FULLFSYNC` takes no argument; the variadic tail is unused.
    // The fd is a valid open file descriptor (the temp file we just wrote).
    let rc = unsafe { fcntl(fd, F_FULLFSYNC) };
    if rc != 0 {
        log_or_eprint(&format!(
            "FilePersister: F_FULLFSYNC failed (rc={rc}); relying on fsync"
        ));
    }
}

/// fsync the parent directory so a rename is durable across power loss.
#[cfg(unix)]
fn fsync_parent(parent: &Path) {
    use std::os::fd::AsRawFd;
    match OpenOptions::new().read(true).open(parent) {
        Ok(dir) => {
            let fd = dir.as_raw_fd();
            unsafe extern "C" {
                fn fsync(fd: std::ffi::c_int) -> std::ffi::c_int;
            }
            // SAFETY: `fd` is a valid open directory file descriptor.
            let rc = unsafe { fsync(fd) };
            if rc != 0 {
                log_or_eprint("FilePersister: parent-dir fsync failed");
            }
        }
        Err(e) => {
            log_or_eprint(&format!(
                "FilePersister: could not open parent dir for fsync: {e}"
            ));
        }
    }
}
