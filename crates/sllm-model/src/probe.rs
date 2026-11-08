use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use sllm_core::config::{EngineConfig, KvType};
use sllm_core::error::SllmError;
use sllm_core::gguf::{self, GgufMeta, PreflightReport};

/// Everything read from one GGUF file without loading weights.
#[derive(Debug, Clone)]
pub struct FileFacts {
    pub meta: GgufMeta,
    pub preflight: PreflightReport,
}

/// Injectable GGUF reader used by [`crate::ModelRegistry`].
///
/// The registry only needs metadata and a compatibility report. Tests use a
/// fake probe so grouping can be exercised without copying real model files;
/// production uses [`FsProbe`].
pub trait GgufProbe: Send + Sync {
    fn probe(&self, path: &Path, bytes: u64, config: &EngineConfig)
    -> Result<FileFacts, SllmError>;
}

/// Filesystem-backed probe with a `(path, size, mtime, ctx, kv)` cache.
///
/// `gguf::read_meta` and `gguf::preflight` both read only the header and the
/// tensor table, never the weights. The cache exists because the registry may
/// be rebuilt on every model-screen refresh and a 12 GB file should not be
/// opened twice for the same unchanged bytes.
#[derive(Debug, Default)]
pub struct FsProbe {
    cache: Mutex<HashMap<PathBuf, CacheEntry>>,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    bytes: u64,
    modified: Option<SystemTime>,
    ctx: u32,
    kv: KvType,
    facts: FileFacts,
}

impl FsProbe {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process-wide probe used by [`crate::ModelRegistry::discover`].
    ///
    /// Keeping it alive between model-screen refreshes avoids re-reading the
    /// same GGUF headers. Entries are invalidated by size, mtime, `ctx` and
    /// `kv`, so edited files are re-read on the next discovery.
    pub fn shared() -> &'static Self {
        static SHARED: OnceLock<FsProbe> = OnceLock::new();
        SHARED.get_or_init(Self::new)
    }

    /// Number of cached files. Useful for diagnostics and tests.
    pub fn cache_len(&self) -> usize {
        match self.cache.lock() {
            Ok(guard) => guard.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }
}

impl GgufProbe for FsProbe {
    fn probe(
        &self,
        path: &Path,
        bytes: u64,
        config: &EngineConfig,
    ) -> Result<FileFacts, SllmError> {
        let modified = std::fs::metadata(path)
            .map_err(|e| SllmError::io(format!("stat {}", path.display()), e))?
            .modified()
            .ok();

        {
            let guard = match self.cache.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(entry) = guard.get(path)
                && entry.bytes == bytes
                && entry.modified == modified
                && entry.ctx == config.ctx
                && entry.kv == config.kv
            {
                return Ok(entry.facts.clone());
            }
        }

        // sllm-core's preflight re-opens the file for metadata. That is still a
        // header-only read, and the result is cached below.
        let meta = gguf::read_meta(path)?;
        let preflight = gguf::preflight(path, bytes, u64::MAX - 4096, config);
        let facts = FileFacts { meta, preflight };

        let mut guard = match self.cache.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.insert(
            path.to_path_buf(),
            CacheEntry {
                bytes,
                modified,
                ctx: config.ctx,
                kv: config.kv,
                facts: facts.clone(),
            },
        );
        Ok(facts)
    }
}
