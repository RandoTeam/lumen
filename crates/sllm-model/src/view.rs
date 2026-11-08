use serde::{Deserialize, Serialize};
use sllm_core::model::Capability;

/// Role of a file inside a pack. Shards are never shown as separate models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileRole {
    Base,
    Shard,
    Lora,
    Mmproj,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub role: FileRole,
    pub name: String,
    /// Path relative to the registry root, always using `/` separators.
    pub path: String,
    pub bytes: u64,
    /// 1-based shard number for split models.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shard: Option<u32>,
}

/// Coarse answer to "will this model fit right now?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// `estimate_mb <= avail_mb`.
    Fits,
    /// Fits only inside the 1 GB safety margin (`estimate_mb <= avail_mb + 1024`).
    Tight,
    /// Clearly does not fit.
    No,
    /// Unknown because preflight failed.
    Unknown,
}

/// Stable status contract consumed by the UI and JNI bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Ready,
    StorageBound,
    Unsupported,
}

/// Human badge label for a capability. The enum itself serializes as
/// `snake_case`; the registry exposes these labels instead so the UI contract
/// stays stable.
pub fn capability_label(cap: Capability) -> &'static str {
    match cap {
        Capability::Text => "Text",
        Capability::Vision => "Vision",
        Capability::Audio => "Audio",
        Capability::Video => "Video",
        Capability::Embedding => "Embedding",
        Capability::Rerank => "Rerank",
        Capability::Diffusion => "Diffusion",
        Capability::Tts => "TTS",
    }
}

/// One model pack evaluated against the memory available right now.
///
/// All field names are camelCase in JSON for the JNI bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    pub id: String,
    pub name: String,
    pub arch: String,
    pub quant: String,
    pub size_bytes: u64,
    pub size_gb: f64,
    pub estimate_mb: u32,
    /// Relative path to the first shard (or the base file).
    pub base_path: String,
    pub files: Vec<FileInfo>,
    pub capabilities: Vec<String>,
    pub chat_template: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    pub layers: u32,
    pub context: u32,
    pub moe: bool,
    pub expert_count: u32,
    pub experts_active: u32,
    pub ternary: bool,
    pub has_lora: bool,
    pub lora_count: usize,
    pub has_mmproj: bool,
    pub shards_total: u32,
    pub shards_present: u32,
    pub status: Status,
    pub fit: Fit,
    /// Short English explanation shown under the model card.
    pub reason: String,
}

/// Full registry JSON for one `nativeModels` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryView {
    pub root: String,
    pub avail_mb: u64,
    pub models: Vec<ModelView>,
}
