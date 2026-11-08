//! The backend-independent engine trait.
//!
//! All methods take `&self` on purpose. The JNI layer holds one
//! `Arc<dyn Engine>` and runs [`Engine::generate`] on a worker thread; the UI
//! thread calls [`Engine::cancel`] on the same value. If `generate` required
//! `&mut self`, cancellation would have to wait for the generation lock and
//! would be useless while a stream is running.

use sllm_core::EngineConfig;

use crate::error::EngineError;
use crate::types::{EngineEvent, EngineStats, GenStats, GenerateRequest, ModelRef, TrimLevel};

/// A chat/inference backend.
///
/// Contract:
///
/// - `load` is idempotent in the sense that loading another model replaces the
///   current one. It must not be called while `generate` is running (`E_BUSY`).
/// - `generate` emits `Started`, zero or more `Token`s, then exactly one
///   `Done` or `Error`. On cancellation it emits `Done` with
///   `stats.canceled == true`; cancellation is not an error. A call rejected
///   before a stream starts returns `Err` and emits no event (`E_BUSY`), or
///   emits one `Error` (`E_CONFIG`, `E_NO_MODEL`).
/// - `cancel` is safe from any thread. It only affects an in-flight
///   generation; calling it while idle is a no-op.
/// - `trim` and `unload` are best-effort and safe from any thread. While a
///   generation is running they are deferred until it finishes rather than
///   invalidating memory under the decoder.
/// - Implementations must be `Send + Sync`.
pub trait Engine: Send + Sync {
    /// Open a model and make the engine `Ready`.
    fn load(&self, model: &ModelRef, config: &EngineConfig) -> Result<(), EngineError>;

    /// Run one generation, streaming events into `sink`.
    ///
    /// This blocks the calling thread; the JNI layer must call it on a worker.
    fn generate(
        &self,
        request: &GenerateRequest,
        sink: &mut dyn FnMut(EngineEvent),
    ) -> Result<GenStats, EngineError>;

    /// Request cancellation of the in-flight generation, if any.
    fn cancel(&self);

    /// Apply an Android-style memory trim.
    fn trim(&self, level: TrimLevel);

    /// Snapshot of the engine state and counters.
    fn stats(&self) -> EngineStats;

    /// Release the model. The engine returns to `Unloaded`.
    fn unload(&self);
}
