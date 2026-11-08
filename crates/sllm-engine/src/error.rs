//! Engine-level errors.
//!
//! Codes are stable strings so they can cross the JNI/JSON boundary unchanged.
//! Engine-level failures use the constants in [`codes`]; errors inherited from
//! `sllm-core` keep their own `E_*` code via [`EngineError::from_core`].

use std::fmt;

use sllm_core::{ErrorCode, SllmError};

/// Stable error codes produced by the engine contract.
pub mod codes {
    /// No model is loaded, or its weights were released by `trim(Complete)`.
    pub const E_NO_MODEL: &str = "E_NO_MODEL";
    /// A generation is already running on this engine.
    pub const E_BUSY: &str = "E_BUSY";
    /// The user stopped the generation. Not a failure; reported for callers
    /// that prefer an error-shaped result.
    pub const E_CANCELED: &str = "E_CANCELED";
    /// Invalid `EngineConfig`, `SamplerConfig` or `ModelRef`.
    pub const E_CONFIG: &str = "E_CONFIG";
    /// A model file could not be read or opened.
    pub const E_IO: &str = "E_IO";
    /// The backend failed to initialise or load weights.
    pub const E_LOAD: &str = "E_LOAD";
    /// The backend failed while decoding.
    pub const E_GENERATE: &str = "E_GENERATE";
}

/// A failure from the engine, suitable for display and for machine handling.
///
/// `code` is a stable `E_*` string. `retryable` is advisory: `E_BUSY` and
/// transient OOM/I/O failures can be retried by the UI, `E_CONFIG` and
/// `E_NO_MODEL` need a user action first.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EngineError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl EngineError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }

    /// Mark whether the caller may retry the same operation.
    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new(codes::E_CONFIG, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(codes::E_IO, message).retryable(true)
    }

    pub fn busy() -> Self {
        Self::new(codes::E_BUSY, "another generation is already running").retryable(true)
    }

    pub fn no_model(message: impl Into<String>) -> Self {
        Self::new(codes::E_NO_MODEL, message)
    }

    pub fn load(message: impl Into<String>) -> Self {
        Self::new(codes::E_LOAD, message).retryable(true)
    }

    pub fn generate(message: impl Into<String>) -> Self {
        Self::new(codes::E_GENERATE, message).retryable(true)
    }

    pub fn canceled() -> Self {
        Self::new(codes::E_CANCELED, "generation canceled")
    }

    pub fn is_canceled(&self) -> bool {
        self.code == codes::E_CANCELED
    }

    /// Convert an `sllm-core` error, preserving its stable code and folding
    /// `detail` into the message shown by the UI.
    pub fn from_core(error: SllmError) -> Self {
        let retryable = matches!(
            error.code,
            ErrorCode::Busy | ErrorCode::LoadNative | ErrorCode::RamLow | ErrorCode::Io
        );
        let message = match error.detail {
            Some(detail) if !detail.trim().is_empty() => {
                format!("{}: {detail}", error.message)
            }
            _ => error.message,
        };
        Self {
            code: error.code.as_str().to_string(),
            message,
            retryable,
        }
    }
}

impl From<SllmError> for EngineError {
    fn from(error: SllmError) -> Self {
        Self::from_core(error)
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for EngineError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_keeps_code_and_detail() {
        let err = EngineError::from(
            SllmError::new(ErrorCode::QuantUnsupported, "quant is not supported")
                .with_detail("PTQ1_0; use Q2_0"),
        );

        assert_eq!(err.code, "E_QUANT_UNSUPPORTED");
        assert!(!err.retryable);
        assert!(err.message.contains("use Q2_0"));
        assert_eq!(
            err.to_string(),
            "E_QUANT_UNSUPPORTED: quant is not supported: PTQ1_0; use Q2_0"
        );
    }

    #[test]
    fn io_core_error_is_retryable() {
        let err = EngineError::from(SllmError::new(ErrorCode::Io, "missing"));
        assert_eq!(err.code, "E_IO");
        assert!(err.retryable);
    }

    #[test]
    fn canceled_is_recognized() {
        assert!(EngineError::canceled().is_canceled());
        assert!(!EngineError::busy().is_canceled());
    }
}
