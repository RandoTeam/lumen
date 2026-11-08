//! Model registry: discovery, packs, compatibility and fit.
//!
//! The registry walks a model directory, groups split shards and companion
//! files (LoRA, mmproj) into one pack per model, dedupes identical copies and
//! evaluates each pack for a given amount of free memory.
//!
//! Everything is pure Rust and testable on the host: the GGUF reader is behind
//! [`GgufProbe`], so grouping tests do not need real multi-gigabyte files.

pub mod error;
pub mod probe;
pub mod registry;
pub mod view;

pub use error::ModelError;
pub use probe::{FileFacts, FsProbe, GgufProbe};
pub use registry::{ModelRegistry, Pack, Registry};
pub use view::{FileInfo, FileRole, Fit, ModelView, RegistryView, Status, capability_label};
