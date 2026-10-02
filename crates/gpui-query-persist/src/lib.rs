//! Reference disk persistence adapter for [`gpui_query`]: [`FilePersister`],
//! an atomic, durable [`Persister`] over one JSON or bincode file, plus a
//! re-exported [`NoopPersister`] for tests and disabled modes.

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

/// Atomic, durable [`Persister`]: each save writes a sibling `O_EXCL` temp file, fsyncs it,
/// renames it over the target, then fsyncs the parent directory, so a crash never leaves a
/// truncated file. Owner-only (`0o600` on Unix). A missing or corrupt file loads as empty; a
/// version mismatch returns [`PersistError::VersionMismatch`]; Windows `ERROR_ACCESS_DENIED`
/// (antivirus, concurrent reader) maps to retryable [`PersistError::Permission`].
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

    /// JSON persister at `<cache_dir>/<app_name>/gpui-query-cache.json`; [`PersistError::BadPath`]
    /// when the OS reports no cache dir. `app_name` is joined as-is, so treat it as trusted.
    pub fn in_cache_dir(app_name: impl AsRef<str>) -> Result<Self, PersistError> {
        let app_name = app_name.as_ref();
        let dir = dirs::cache_dir().ok_or_else(|| {
            PersistError::BadPath(format!("no OS cache dir available for app {app_name:?}"))
        })?;
        Ok(Self::json(dir.join(app_name).join("gpui-query-cache.json")))
    }

    /// The on-disk path this persister writes to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn write_atomic(&self, snapshot: &PersistSnapshot) -> Result<(), PersistError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| PersistError::Permission("write lock poisoned".to_string()))?;

        let parent = effective_parent(&self.path);
        fs::create_dir_all(parent)?;

        let bytes: Vec<u8> = match self.format {
            PersistFormat::Json => serde_json::to_vec(snapshot)?,
            PersistFormat::Bincode => {
                let adapter = BincodeSnapshot::from_snapshot(snapshot)?;
                bincode::serialize(&adapter).map_err(|e| {
                    use serde::ser::Error as _;
                    PersistError::Serialize(serde_json::Error::custom(e.to_string()))
                })?
            }
        };

        let mut tmp = tempfile::Builder::new()
            .prefix(
                self.path
                    .file_name()
                    .map(Path::new)
                    .unwrap_or_else(|| Path::new("cache")),
            )
            .suffix(".tmp")
            .tempfile_in(parent)?;
        tmp.write_all(&bytes)?;
        tmp.as_file().sync_all()?;
        #[cfg(target_os = "macos")]
        try_fullfsync(tmp.as_file());
        tmp.persist(&self.path).map_err(|persist_err| {
            let io_err = persist_err.error;
            let denied = io_err.kind() == std::io::ErrorKind::PermissionDenied
                || is_windows_access_denied(io_err.raw_os_error());
            if denied {
                PersistError::Permission(format!(
                    "atomic persist of cache file was denied (retryable): {io_err}"
                ))
            } else {
                PersistError::Io(io_err)
            }
        })?;

        #[cfg(unix)]
        fsync_parent(parent);

        Ok(())
    }

    /// Lock-free: the atomic rename means a concurrent save only swaps in a complete file.
    fn read_tolerant(&self) -> Result<PersistSnapshot, PersistError> {
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PersistSnapshot::new());
            }
            Err(e) => return Err(e.into()),
        };

        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        let (label, parsed): (&str, Result<PersistSnapshot, String>) = match self.format {
            PersistFormat::Json => (
                "JSON",
                serde_json::from_slice(&buf).map_err(|e| e.to_string()),
            ),
            PersistFormat::Bincode => ("bincode", bincode_load(&buf)),
        };
        let snapshot = match parsed {
            Ok(s) => s,
            Err(detail) => {
                eprintln!(
                    "FilePersister: corrupt {label} cache at {}: {detail}; treating as empty",
                    self.path.display()
                );
                return Ok(PersistSnapshot::new());
            }
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

pub use gpui_query::client::NoopPersister;

/// bincode cannot drive `serde_json::Value`'s `deserialize_any`; `value` and
/// `meta` are carried as JSON strings. Lossless.
#[derive(serde::Serialize, serde::Deserialize)]
struct BincodeSnapshot {
    entries: HashMap<String, BincodeEntry>,
    version: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct BincodeEntry {
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
                    meta_json: e.meta.as_ref().map(serde_json::to_string).transpose()?,
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
            let meta = e.meta_json.map(|m| serde_json::from_str(&m)).transpose()?;
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

fn bincode_load(buf: &[u8]) -> Result<PersistSnapshot, String> {
    let adapter: BincodeSnapshot = bincode::deserialize(buf).map_err(|e| e.to_string())?;
    adapter.into_snapshot().map_err(|e| e.to_string())
}

/// std maps `ERROR_ACCESS_DENIED` (5) to `PermissionDenied`; errors built
/// via `from_raw_os_error` on older toolchains may not be.
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

/// macOS `F_FULLFSYNC` also flushes the drive's write cache, unlike plain
/// `fsync`. Best-effort: the save already succeeded via the earlier `sync_all`.
#[cfg(target_os = "macos")]
fn try_fullfsync(file: &File) {
    // F_FULLFSYNC = 0x00008027 (fcntl.h on Darwin); extern declared here to avoid a libc dep.
    unsafe extern "C" {
        fn fcntl(fd: std::os::fd::RawFd, cmd: std::ffi::c_int, ...) -> std::ffi::c_int;
    }
    const F_FULLFSYNC: std::ffi::c_int = 0x00008027;
    use std::os::fd::AsRawFd;
    // SAFETY: no variadic argument is passed and the fd is the temp file we just wrote.
    let rc = unsafe { fcntl(file.as_raw_fd(), F_FULLFSYNC) };
    if rc != 0 {
        eprintln!("FilePersister: F_FULLFSYNC failed (rc={rc}); relying on fsync");
    }
}

/// fsync the parent directory so the rename is durable across power loss.
#[cfg(unix)]
fn fsync_parent(parent: &Path) {
    match OpenOptions::new().read(true).open(parent) {
        Ok(dir) => {
            if let Err(e) = dir.sync_all() {
                eprintln!("FilePersister: parent-dir fsync failed: {e}");
            }
        }
        Err(e) => {
            eprintln!("FilePersister: could not open parent dir for fsync: {e}");
        }
    }
}

/// A bare filename has `parent() == Some("")`, which `open`/`create_dir_all` reject or skip.
fn effective_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bincode_corrupt_inner_json_is_tolerated() {
        let adapter = BincodeSnapshot {
            entries: HashMap::from([(
                "users::42".to_string(),
                BincodeEntry {
                    value_json: "{ this is not valid json".to_string(),
                    cached_at: 1_700_000_000_000,
                    cache_policy: CachePolicy::NoCache,
                    meta_json: None,
                },
            )]),
            version: PERSIST_VERSION,
        };
        let bytes = bincode::serialize(&adapter).expect("serialize adapter frame");
        assert!(
            bincode_load(&bytes).is_err(),
            "corrupt inner JSON must surface as a load error"
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cache.bin");
        std::fs::write(&path, &bytes).expect("write framed payload");
        let p = FilePersister::bincode(&path);
        let loaded = pollster::block_on(p.load()).expect("tolerant load");
        assert!(
            loaded.entries.is_empty(),
            "corrupt inner JSON -> empty snapshot"
        );
    }

    #[test]
    fn bincode_corrupt_meta_json_is_tolerated() {
        let adapter = BincodeSnapshot {
            entries: HashMap::from([(
                "users::42".to_string(),
                BincodeEntry {
                    value_json: "null".to_string(),
                    cached_at: 0,
                    cache_policy: CachePolicy::NoCache,
                    meta_json: Some("{ this is not valid json".to_string()),
                },
            )]),
            version: PERSIST_VERSION,
        };
        let bytes = bincode::serialize(&adapter).expect("serialize adapter frame");
        assert!(bincode_load(&bytes).is_err());
    }

    #[test]
    fn effective_parent_of_bare_filename_is_dot() {
        assert_eq!(effective_parent(Path::new("cache.json")), Path::new("."));
        assert_eq!(
            effective_parent(Path::new("a/b/cache.json")),
            Path::new("a/b")
        );
    }
}

#[cfg(doctest)]
mod readme_doctests {
    #[doc = include_str!("../README.md")]
    struct Readme;
}
