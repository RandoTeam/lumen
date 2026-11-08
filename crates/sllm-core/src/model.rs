use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, SllmError};
use crate::gguf::{self, GgufMeta, PreflightReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Text,
    Vision,
    Audio,
    Video,
    Embedding,
    Rerank,
    Diffusion,
    Tts,
}

#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub meta: GgufMeta,
    pub preflight: Option<PreflightReport>,
}

impl ModelEntry {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, SllmError> {
        let path = path.into();
        let bytes = std::fs::metadata(&path)
            .map_err(|e| SllmError::io(format!("stat {}", path.display()), e))?
            .len();
        let meta = gguf::read_meta(&path)?;
        let name = display_name(&path, &meta);
        Ok(Self {
            path,
            name,
            bytes,
            meta,
            preflight: None,
        })
    }

    pub fn quant(&self) -> String {
        gguf::quant_of(&self.meta)
    }

    pub fn is_moe(&self) -> bool {
        self.meta.is_moe()
    }

    pub fn size_gb(&self) -> f64 {
        self.bytes as f64 / 1024.0 / 1024.0 / 1024.0
    }

    pub fn kind(&self) -> String {
        if self.meta.is_moe() {
            format!(
                "MoE {}e / {} active",
                self.meta.expert_count.unwrap_or(0),
                self.meta.expert_used.unwrap_or(0)
            )
        } else if let Some(arch) = &self.meta.architecture {
            arch.trim_end_matches("_moe").to_string()
        } else {
            "GGUF".to_string()
        }
    }
}

fn display_name(path: &Path, meta: &GgufMeta) -> String {
    if meta.architecture.as_deref() == Some("clover_moe") {
        return "Clover 35B-A3B".to_string();
    }
    if let Some(name) = meta.name.as_deref() {
        if !name.trim().is_empty() && !name.ends_with(" Src") {
            return name.to_string();
        }
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string()
}

/// Scan a directory for chat-capable GGUF files. Adapters, mmproj and
/// non-first split shards are hidden from the main model list.
pub fn scan_dir(dir: &Path) -> Result<Vec<ModelEntry>, SllmError> {
    let mut out = Vec::new();
    collect(dir, &mut out)?;
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    Ok(out)
}

fn collect(dir: &Path, out: &mut Vec<ModelEntry>) -> Result<(), SllmError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(SllmError::io(format!("read_dir {}", dir.display()), e)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| SllmError::io("read_dir entry", e))?;
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out)?;
            continue;
        }
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gguf"))
        {
            continue;
        }
        let Ok(model) = ModelEntry::load(&path) else {
            continue;
        };
        if model.meta.is_adapter() || model.meta.is_mmproj() {
            continue;
        }
        if model.meta.is_split() && !model.meta.is_first_shard() {
            continue;
        }
        out.push(model);
    }
    Ok(())
}

pub fn require_chat_model(model: &ModelEntry) -> Result<(), SllmError> {
    if model.meta.is_adapter() {
        return Err(SllmError::new(
            ErrorCode::LoraNotModel,
            "adapter is not a model",
        ));
    }
    if model.meta.is_mmproj() {
        return Err(SllmError::new(
            ErrorCode::MmprojNotModel,
            "mmproj is not a model",
        ));
    }
    Ok(())
}
