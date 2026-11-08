//! Core, UI-agnostic types for SnapdragonLLM.
//!
//! This crate deliberately has no Android or UI dependency. The app crate
//! supplies a `DeviceProfile`, paths and a JSONL sink; the native engine crate
//! implements inference. Keeping these types pure makes them testable on the
//! host.

pub mod config;
pub mod device;
pub mod error;
pub mod gguf;
pub mod log;
pub mod model;
pub mod pack;

pub use config::{Backend, EngineConfig, GenConfig, KvType};
pub use device::{CpuFeatures, DeviceProfile};
pub use error::{ErrorCode, SllmError};
pub use gguf::{GgufMeta, PreflightReport};
pub use model::{Capability, ModelEntry};
pub use pack::{LoraFile, ModelPack, PackFile};
