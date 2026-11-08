use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, SllmError};
use crate::model::Capability;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackFile {
    pub role: String,
    pub name: String,
    pub bytes: u64,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoraFile {
    pub file: String,
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default)]
    pub sha256: Option<String>,
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct License {
    pub spdx: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommended {
    pub ctx: u32,
    pub batch: u32,
    #[serde(default = "default_kv")]
    pub kv: String,
}

fn default_kv() -> String {
    "q8_0".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPack {
    #[serde(default = "schema")]
    pub schema: u32,
    pub id: String,
    #[serde(default = "version")]
    pub version: u32,
    pub base: PackFile,
    #[serde(default)]
    pub mmproj: Option<PackFile>,
    #[serde(default)]
    pub lora: Vec<LoraFile>,
    #[serde(default)]
    pub chat_template: Option<String>,
    #[serde(default)]
    pub license: Option<License>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub min_ram_mb: u32,
    #[serde(default)]
    pub recommended: Option<Recommended>,
}

fn schema() -> u32 {
    1
}

fn version() -> u32 {
    1
}

impl ModelPack {
    pub fn load(path: &Path) -> Result<Self, SllmError> {
        let data = std::fs::read(path)
            .map_err(|e| SllmError::io(format!("read {}", path.display()), e))?;
        serde_json::from_slice(&data).map_err(|e| {
            SllmError::new(ErrorCode::Config, "invalid pack.json").with_detail(e.to_string())
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), SllmError> {
        let data = serde_json::to_vec_pretty(self).map_err(|e| {
            SllmError::new(ErrorCode::Config, "serialize pack").with_detail(e.to_string())
        })?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SllmError::io("create pack dir", e))?;
        }
        std::fs::write(path, data)
            .map_err(|e| SllmError::io(format!("write {}", path.display()), e))
    }

    pub fn dir(&self, root: &Path) -> PathBuf {
        root.join(&self.id)
    }

    pub fn base_path(&self, root: &Path) -> PathBuf {
        self.dir(root).join(&self.base.name)
    }

    pub fn mmproj_path(&self, root: &Path) -> Option<PathBuf> {
        self.mmproj.as_ref().map(|m| self.dir(root).join(&m.name))
    }

    pub fn lora_paths(&self, root: &Path) -> Vec<PathBuf> {
        self.lora
            .iter()
            .map(|l| self.dir(root).join(&l.file))
            .collect()
    }

    pub fn validate(&self) -> Result<(), SllmError> {
        if self.id.trim().is_empty() {
            return Err(SllmError::new(ErrorCode::Config, "pack id is empty"));
        }
        if self.base.name.trim().is_empty() {
            return Err(SllmError::new(ErrorCode::Config, "pack base is empty"));
        }
        if !(0.0..=10.0).contains(&self.lora.iter().map(|l| l.scale).fold(1.0, f32::max)) {
            return Err(SllmError::new(ErrorCode::Config, "LoRA scale out of range"));
        }
        Ok(())
    }
}
