use serde::{Deserialize, Serialize};

use crate::device::DeviceProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    #[default]
    Auto,
    Cpu,
    Vulkan,
    OpenCl,
    Hexagon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KvType {
    F16,
    #[default]
    Q8_0,
    Q4_0,
}

impl KvType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::F16 => "f16",
            Self::Q8_0 => "q8_0",
            Self::Q4_0 => "q4_0",
        }
    }

    /// Bytes per stored element, used by the memory estimate.
    pub const fn bytes_per_element(self) -> f64 {
        match self {
            Self::F16 => 2.0,
            Self::Q8_0 => 1.0625,
            Self::Q4_0 => 0.5625,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    pub threads: u32,
    pub ctx: u32,
    pub batch: u32,
    pub kv: KvType,
    pub mmap: bool,
    pub mlock: bool,
    pub flash_attn: bool,
    pub backend: Backend,
    pub gpu_layers: u32,
    pub expert_cache_mb: u32,
}

impl EngineConfig {
    /// Conservative default for a known device. The user can override every
    /// field from Settings.
    pub fn for_device(device: &DeviceProfile) -> Self {
        Self {
            threads: device.recommended_threads(),
            ctx: device.recommended_ctx(),
            batch: device.recommended_batch(),
            kv: KvType::Q8_0,
            mmap: true,
            mlock: false,
            flash_attn: true,
            backend: Backend::Cpu,
            gpu_layers: 0,
            // Device-side expert cache is a no-op (and an error) on CPU.
            expert_cache_mb: 0,
        }
    }

    pub fn validate(&self) -> Result<(), crate::error::SllmError> {
        if self.threads == 0 {
            return Err(crate::error::SllmError::new(
                crate::error::ErrorCode::Config,
                "threads must be >= 1",
            ));
        }
        if self.ctx < 512 {
            return Err(crate::error::SllmError::new(
                crate::error::ErrorCode::Config,
                "ctx must be >= 512",
            ));
        }
        if self.batch == 0 {
            return Err(crate::error::SllmError::new(
                crate::error::ErrorCode::Config,
                "batch must be >= 1",
            ));
        }
        Ok(())
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            threads: 4,
            ctx: 4096,
            batch: 128,
            kv: KvType::Q8_0,
            mmap: true,
            mlock: false,
            flash_attn: true,
            backend: Backend::Cpu,
            gpu_layers: 0,
            expert_cache_mb: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenConfig {
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub min_p: f32,
    pub repeat_penalty: f32,
    pub seed: i64,
}

impl Default for GenConfig {
    fn default() -> Self {
        Self {
            max_tokens: 512,
            temperature: 0.7,
            top_p: 0.95,
            top_k: 40,
            min_p: 0.05,
            repeat_penalty: 1.1,
            seed: -1,
        }
    }
}
