use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

use crate::error::{ErrorCode, SllmError};

/// Append-only JSONL log. One line per request/event. Rotates at 8 MiB.
pub struct JsonlLogger {
    path: PathBuf,
    lock: Mutex<()>,
}

impl JsonlLogger {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append<T: Serialize>(&self, event: &T) -> Result<(), SllmError> {
        let line = serde_json::to_string(event).map_err(|e| {
            SllmError::new(ErrorCode::Io, "serialize log").with_detail(e.to_string())
        })?;
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SllmError::io("create log dir", e))?;
        }
        if std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0) > 8 * 1024 * 1024 {
            let old = self.path.with_extension("1.jsonl");
            let _ = std::fs::rename(&self.path, old);
        }
        let mut file: File = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| SllmError::io(format!("open {}", self.path.display()), e))?;
        writeln!(file, "{line}").map_err(|e| SllmError::io("write log", e))
    }

    pub fn tail(&self, max_bytes: usize) -> String {
        let data = std::fs::read(&self.path).unwrap_or_default();
        let start = data.len().saturating_sub(max_bytes);
        String::from_utf8_lossy(&data[start..]).into_owned()
    }

    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
