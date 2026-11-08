use std::fmt;

/// Stable, user-facing error codes. The UI maps these to short messages and
/// actions; logs keep the code plus the original detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    GgufHeader,
    GgufTruncated,
    QuantUnsupported,
    ArchUnknown,
    TemplateFallback,
    SplitNotFirst,
    LoraNotModel,
    MmprojNotModel,
    RamLow,
    LoadNative,
    PreflightBlocked,
    ContextClamped,
    KvUnsupported,
    NoModel,
    Busy,
    Cancelled,
    Io,
    Config,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GgufHeader => "E_GGUF_HEADER",
            Self::GgufTruncated => "E_GGUF_TRUNCATED",
            Self::QuantUnsupported => "E_QUANT_UNSUPPORTED",
            Self::ArchUnknown => "E_ARCH_UNKNOWN",
            Self::TemplateFallback => "E_TEMPLATE_FALLBACK",
            Self::SplitNotFirst => "E_SPLIT_NOT_FIRST",
            Self::LoraNotModel => "E_LORA_NOT_MODEL",
            Self::MmprojNotModel => "E_MMPROJ_NOT_MODEL",
            Self::RamLow => "E_RAM_LOW",
            Self::LoadNative => "E_LOAD_NATIVE",
            Self::PreflightBlocked => "E_PREFLIGHT_BLOCKED",
            Self::ContextClamped => "E_CTX_CLAMPED",
            Self::KvUnsupported => "E_KV_UNSUPPORTED",
            Self::NoModel => "E_NO_MODEL",
            Self::Busy => "E_BUSY",
            Self::Cancelled => "E_CANCELLED",
            Self::Io => "E_IO",
            Self::Config => "E_CONFIG",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct SllmError {
    pub code: ErrorCode,
    pub message: String,
    pub detail: Option<String>,
}

impl SllmError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn io(context: impl Into<String>, error: std::io::Error) -> Self {
        Self::new(ErrorCode::Io, context).with_detail(error.to_string())
    }
}
