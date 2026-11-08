//! Engine API: load, generate, stream, cancel, trim, stats.
//!
//! This crate owns the stable inference contract and a mock backend. It has no
//! Android, JNI or llama.cpp dependency; the real backend will implement
//! [`Engine`] in `sllm-llama-sys`/`sllm-app` without changing the UI-facing
//! types.
//!
//! ```no_run
//! use std::sync::Arc;
//! use std::time::Duration;
//! use sllm_engine::{
//!     Engine, EngineConfig, EngineEvent, GenerateRequest, Message, MockEngine, ModelRef,
//!     TrimLevel,
//! };
//!
//! let engine: Arc<dyn Engine> = Arc::new(MockEngine::new().with_delay(Duration::ZERO));
//! engine
//!     .load(&ModelRef::new("qwen", "models/qwen.gguf"), &EngineConfig::default())
//!     .unwrap();
//!
//! let request = GenerateRequest::new(vec![Message::user("hi")]);
//! let stats = engine
//!     .generate(&request, &mut |event| match event {
//!         EngineEvent::Token { text, .. } => print!("{text}"),
//!         EngineEvent::Done { .. } => println!(),
//!         _ => {}
//!     })
//!     .unwrap();
//! assert!(stats.mock);
//! engine.trim(TrimLevel::Complete);
//! assert_eq!(engine.stats().state, sllm_engine::EngineState::Trimmed);
//! ```

pub mod engine;
pub mod error;
#[cfg(feature = "llama")]
pub mod llama;
pub mod mock;
mod stop;
pub mod types;

pub use engine::Engine;
pub use error::{EngineError, codes};
#[cfg(feature = "llama")]
pub use llama::LlamaEngine;
pub use mock::MockEngine;
pub use types::{
    EngineEvent, EngineState, EngineStats, GenStats, GenerateRequest, LoraRef, Message, ModelRef,
    Role, SamplerConfig, TrimLevel,
};

// One source of truth for configuration shared with `sllm-core`.
pub use sllm_core::config::{Backend, EngineConfig, KvType};
pub use sllm_core::{ErrorCode, SllmError};
