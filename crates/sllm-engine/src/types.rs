//! Stable, UI-agnostic data types for the inference engine.
//!
//! Nothing here touches Android, JNI or llama.cpp. `EngineConfig`, `Backend`
//! and `KvType` are re-exported from `sllm-core` so the app and the engine
//! share one configuration type.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::EngineError;

/// A LoRA adapter to apply at runtime. Never merged into the base weights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoraRef {
    pub path: PathBuf,
    #[serde(default = "one")]
    pub scale: f32,
}

fn one() -> f32 {
    1.0
}

impl LoraRef {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            scale: 1.0,
        }
    }

    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }
}

/// Everything the engine needs to open a model.
///
/// This lives in `sllm-engine` until `sllm-model` grows the real registry
/// type. It is deliberately small: the UI's `ModelSummary` is a different,
/// larger type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    /// Stable id used by the UI and logs; may be the pack id.
    pub id: String,
    /// First shard of the GGUF (or the adapter-compatible base model).
    pub path: PathBuf,
    /// Human name for logs; the UI already has its own summary.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub loras: Vec<LoraRef>,
    #[serde(default)]
    pub mmproj: Option<PathBuf>,
}

impl ModelRef {
    pub fn new(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            name: None,
            loras: Vec::new(),
            mmproj: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_lora(mut self, lora: LoraRef) -> Self {
        self.loras.push(lora);
        self
    }

    pub fn display_name(&self) -> String {
        if let Some(name) = self.name.as_deref()
            && !name.trim().is_empty()
        {
            return name.to_string();
        }
        self.path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| self.id.clone())
    }
}

/// Chat role in a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

/// One chat turn. The engine applies the model's chat template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }
}

/// Sampler chain settings. `temperature == 0.0` means greedy/argmax.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplerConfig {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub min_p: f32,
    pub repeat_penalty: f32,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.95,
            top_k: 40,
            min_p: 0.05,
            repeat_penalty: 1.1,
        }
    }
}

impl SamplerConfig {
    pub fn validate(&self) -> Result<(), EngineError> {
        if !self.temperature.is_finite() || self.temperature < 0.0 {
            return Err(EngineError::config(
                "temperature must be finite and >= 0 (0 = greedy)",
            ));
        }
        if !self.top_p.is_finite() || self.top_p <= 0.0 || self.top_p > 1.0 {
            return Err(EngineError::config("top_p must be in (0, 1]"));
        }
        if !self.min_p.is_finite() || !(0.0..=1.0).contains(&self.min_p) {
            return Err(EngineError::config("min_p must be in [0, 1]"));
        }
        if !self.repeat_penalty.is_finite() || self.repeat_penalty <= 0.0 {
            return Err(EngineError::config("repeat_penalty must be > 0"));
        }
        // top_k == 0 disables the top-k stage, matching llama.cpp.
        Ok(())
    }
}

/// One generation request. `seed` is echoed in [`GenStats`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateRequest {
    pub messages: Vec<Message>,
    pub sampler: SamplerConfig,
    pub max_tokens: u32,
    /// Stop strings. The matching text is never emitted to the sink.
    #[serde(default)]
    pub stop: Vec<String>,
    /// `-1` asks the backend for a random seed.
    #[serde(default = "minus_one")]
    pub seed: i64,
}

fn minus_one() -> i64 {
    -1
}

impl Default for GenerateRequest {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            sampler: SamplerConfig::default(),
            max_tokens: 512,
            stop: Vec::new(),
            seed: -1,
        }
    }
}

impl GenerateRequest {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            messages,
            ..Self::default()
        }
    }

    pub fn user_prompt(prompt: impl Into<String>) -> Self {
        Self::new(vec![Message::user(prompt)])
    }

    pub fn validate(&self) -> Result<(), EngineError> {
        self.sampler.validate()
    }
}

/// Streaming events. A stream that starts emits `Started`, then zero or more
/// `Token`s, then exactly one terminal `Done` or `Error`. A call rejected
/// before a stream starts (`E_BUSY`) returns only `Err`; `E_CONFIG` and
/// `E_NO_MODEL` emit a single `Error` and also return `Err`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "event",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EngineEvent {
    /// Prompt accepted; `prompt_tokens` is the backend's count.
    Started { prompt_tokens: usize },
    /// A piece of generated text. Concatenating all `text` fields yields the
    /// visible completion (minus any stop string).
    Token { text: String, id: u32 },
    /// Generation finished, normally or by cancellation.
    Done { stats: GenStats },
    /// Generation failed. The same error is also returned from `generate`.
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
}

impl EngineEvent {
    pub fn from_error(error: &EngineError) -> Self {
        Self::Error {
            code: error.code.clone(),
            message: error.message.clone(),
            retryable: error.retryable,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Error { .. })
    }
}

/// Per-generation measurements.
///
/// `mock` is `true` for the mock backend. Mock numbers are wall-clock values
/// from the simulated stream and must never be presented as device tok/s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenStats {
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
    /// Time from `generate` entry to the first generated token.
    pub ttft_ms: u64,
    /// Time spent on prompt processing (before the first decode step).
    pub prefill_ms: u64,
    /// Time spent decoding after the first token.
    pub decode_ms: u64,
    pub tok_per_s: f32,
    /// Seed echoed from the request; `-1` means "backend chose".
    pub seed: i64,
    /// True when the user stopped the generation.
    pub canceled: bool,
    /// True when these numbers come from `MockEngine`, not a real model.
    pub mock: bool,
}

/// Android `onTrimMemory`-shaped levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimLevel {
    /// All caches resident.
    #[default]
    None,
    /// Drop transient buffers (image cache, scratch).
    Moderate,
    /// Drop KV/recurrent state; weights stay resident.
    RunningLow,
    /// Release weights; keep the model reference and preflight metadata.
    Complete,
}

impl TrimLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Moderate => "moderate",
            Self::RunningLow => "running_low",
            Self::Complete => "complete",
        }
    }
}

/// Coarse engine lifecycle state, exposed by [`EngineStats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    /// No model, or `unload` was called.
    Unloaded,
    /// Weights resident and generation possible.
    Ready,
    /// A generation is in flight.
    Generating,
    /// Weights released by `trim(Complete)`; a new `load` is required before
    /// generation. Metadata and the model id are retained.
    Trimmed,
}

/// Snapshot of engine state, returned by [`crate::Engine::stats`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStats {
    pub state: EngineState,
    pub model_id: Option<String>,
    /// Highest trim level applied since the last load.
    pub trim: TrimLevel,
    /// True when weights are gone but the model reference is kept
    /// (`Trimmed`).
    pub needs_reload: bool,
    /// Generations accepted since the last load, including canceled ones.
    pub requests: u64,
    /// Tokens generated since the last load.
    pub generated_tokens: u64,
    /// Most recent generation stats, if any.
    pub last: Option<GenStats>,
    /// True for the mock backend.
    pub mock: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampler_rejects_invalid_ranges() {
        let sampler = SamplerConfig {
            temperature: -1.0,
            ..SamplerConfig::default()
        };
        assert_eq!(sampler.validate().unwrap_err().code, "E_CONFIG");

        let sampler = SamplerConfig {
            top_p: 1.5,
            ..SamplerConfig::default()
        };
        assert_eq!(sampler.validate().unwrap_err().code, "E_CONFIG");

        let sampler = SamplerConfig {
            repeat_penalty: 0.0,
            ..SamplerConfig::default()
        };
        assert_eq!(sampler.validate().unwrap_err().code, "E_CONFIG");
    }

    #[test]
    fn greedy_is_valid() {
        let sampler = SamplerConfig {
            temperature: 0.0,
            ..SamplerConfig::default()
        };
        assert!(sampler.validate().is_ok());
    }

    #[test]
    fn model_display_name_falls_back_to_file_stem() {
        let model = ModelRef::new("qwen", "models/qwen2.5-0.5b-q4_k_m.gguf");
        assert_eq!(model.display_name(), "qwen2.5-0.5b-q4_k_m");
        assert_eq!(model.with_name("Qwen 0.5B").display_name(), "Qwen 0.5B");
    }

    #[test]
    fn trim_levels_are_ordered() {
        assert!(TrimLevel::None < TrimLevel::Moderate);
        assert!(TrimLevel::Moderate < TrimLevel::RunningLow);
        assert!(TrimLevel::RunningLow < TrimLevel::Complete);
    }

    #[test]
    fn event_json_shape_is_jni_friendly() {
        let event = EngineEvent::Token {
            text: "hello".to_string(),
            id: 7,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "token");
        assert_eq!(json["text"], "hello");
        assert_eq!(json["id"], 7);

        let stats = GenStats {
            prompt_tokens: 10,
            generated_tokens: 3,
            ttft_ms: 40,
            prefill_ms: 20,
            decode_ms: 60,
            tok_per_s: 50.0,
            seed: 42,
            canceled: false,
            mock: true,
        };
        let done = serde_json::to_value(EngineEvent::Done {
            stats: stats.clone(),
        })
        .unwrap();
        assert_eq!(done["event"], "done");
        assert_eq!(done["stats"]["promptTokens"], 10);
        assert_eq!(done["stats"]["generatedTokens"], 3);
        assert_eq!(done["stats"]["ttftMs"], 40);
        assert_eq!(done["stats"]["tokPerS"], 50.0);
        assert_eq!(done["stats"]["mock"], true);

        let request = GenerateRequest::user_prompt("hi");
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["maxTokens"], 512);
        assert!((json["sampler"]["topP"].as_f64().unwrap() - 0.95).abs() < 1e-6);
        assert!((json["sampler"]["repeatPenalty"].as_f64().unwrap() - 1.1).abs() < 1e-6);
    }
}
