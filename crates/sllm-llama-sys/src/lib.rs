//! Minimal, honest FFI layer for the pinned llama.cpp fork.
//!
//! The crate builds llama.cpp with CMake (static by default) and a small C++ shim,
//! then exposes a safe, deliberately thin Rust API. It is not a general-purpose
//! llama.cpp wrapper: only what the SnapdragonLLM engine needs to prove the
//! toolchain is wired up (version, model/context lifetime, tokenize, decode,
//! sampling) is exported.
//!
//! Build with `SLLM_SYS_SKIP_BUILD=1` to compile the API surface without a native
//! build. Every fallible call then returns [`ErrorKind::NotBuilt`].
//!
//! ```no_run
//! use sllm_llama_sys::{ContextOptions, Model, ModelOptions};
//!
//! let model = Model::load_with("model.gguf", ModelOptions::default())?;
//! let mut ctx = model.context(&ContextOptions::default())?;
//! let tokens = ctx.tokenize("Hello", true, false)?;
//! ctx.decode(&tokens, 0, 0, false)?;
//! let logits = ctx.logits();
//! # Ok::<(), sllm_llama_sys::Error>(())
//! ```

#![cfg_attr(
    not(sllm_sys_skip_build),
    allow(
        non_camel_case_types,
        non_snake_case,
        non_upper_case_globals,
        dead_code,
        clippy::all
    )
)]

use std::fmt;

#[cfg(not(sllm_sys_skip_build))]
pub mod raw {
    //! Raw bindgen output for `llama.h`, `ggml.h`, `ggml-backend.h` and the shim.
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

#[cfg(not(sllm_sys_skip_build))]
#[path = "real.rs"]
mod imp;

#[cfg(sllm_sys_skip_build)]
#[path = "stub.rs"]
mod imp;

pub use imp::{Context, Model, Sampler};

/// Target platform the native libraries were built for.
pub const PLATFORM: &str = if cfg!(target_os = "android") {
    "android"
} else {
    "host"
};

/// `true` when compiled for Android.
pub const fn is_android() -> bool {
    cfg!(target_os = "android")
}

/// Best-effort classification of a shim error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// A caller passed an invalid pointer or value.
    InvalidArgument,
    /// A C++ allocation failed.
    OutOfMemory,
    /// A C++ exception crossed the shim and was caught.
    Exception,
    /// llama.cpp reported a failure (model load, decode, sampler, ...).
    State,
    /// The crate was compiled with `SLLM_SYS_SKIP_BUILD=1`.
    NotBuilt,
}

impl ErrorKind {
    #[allow(dead_code)]
    fn from_code(code: i32) -> Self {
        match code {
            -1 => Self::InvalidArgument,
            -2 => Self::OutOfMemory,
            -3 => Self::Exception,
            -4 | 1.. => Self::State,
            _ => Self::State,
        }
    }
}

/// Error returned by the safe wrapper.
#[derive(Clone, Debug)]
pub struct Error {
    code: i32,
    kind: ErrorKind,
    message: String,
}

impl Error {
    #[allow(dead_code)]
    pub(crate) fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            kind: ErrorKind::from_code(code),
            message: message.into(),
        }
    }

    pub(crate) fn not_built() -> Self {
        Self {
            code: 0,
            kind: ErrorKind::NotBuilt,
            message: "sllm-llama-sys was compiled with SLLM_SYS_SKIP_BUILD=1; \
                      rebuild without it to link llama.cpp"
                .to_string(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(-1, message)
    }

    /// Raw code from the shim (or a llama.cpp return value for decode).
    pub fn code(&self) -> i32 {
        self.code
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == ErrorKind::NotBuilt {
            return f.write_str(&self.message);
        }
        write!(
            f,
            "llama.cpp error {} ({:?}): {}",
            self.code, self.kind, self.message
        )
    }
}

impl std::error::Error for Error {}

/// Model loading mode, mirroring `enum llama_load_mode` from the pinned fork.
pub const LOAD_MODE_AUTO: i32 = -1;
/// No mmap, no mlock.
pub const LOAD_MODE_NONE: i32 = 0;
/// Memory-map the model file.
pub const LOAD_MODE_MMAP: i32 = 1;
/// Force the model to stay resident in RAM.
pub const LOAD_MODE_MLOCK: i32 = 2;
/// Memory-map and lock the model.
pub const LOAD_MODE_MMAP_MLOCK: i32 = 3;
/// Use direct I/O where available.
pub const LOAD_MODE_DIRECT_IO: i32 = 4;

/// `ggml_type` values accepted by [`ContextOptions::type_k`]/`type_v`.
pub const GGML_TYPE_F16: i32 = 1;
/// `ggml_type` value for 4-bit quantised KV.
pub const GGML_TYPE_Q4_0: i32 = 2;
/// `ggml_type` value for 8-bit quantised KV.
pub const GGML_TYPE_Q8_0: i32 = 8;

/// Options for [`Model::load_with`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelOptions {
    /// Number of layers to offload to the GPU. 0 keeps everything on the CPU.
    pub n_gpu_layers: i32,
    /// One of the `LOAD_MODE_*` constants.
    pub load_mode: i32,
}

/// Options for [`Model::context`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextOptions {
    /// Context size, 0 means "from the model".
    pub n_ctx: u32,
    /// Logical batch size accepted by decode.
    pub n_batch: u32,
    /// Physical micro-batch size.
    pub n_ubatch: u32,
    /// Threads used for single-token decode.
    pub n_threads: i32,
    /// Threads used for prompt/batch processing.
    pub n_threads_batch: i32,
    /// KV cache type for K, as `ggml_type` (`1` = F16 by llama.cpp default).
    pub type_k: i32,
    /// KV cache type for V, as `ggml_type`.
    pub type_v: i32,
    /// Offload K/Q/V operations to the GPU.
    pub offload_kqv: bool,
    /// `llama_flash_attn_type`: -1 auto, 0 disabled, 1 enabled.
    pub flash_attn_type: i32,
}

/// Sampler chain configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SamplerConfig {
    /// Top-K cutoff, <= 0 disables the stage.
    pub top_k: i32,
    /// Nucleus probability, <= 0 or >= 1 disables the stage.
    pub top_p: f32,
    /// Minimum probability, <= 0 disables the stage.
    pub min_p: f32,
    /// Temperature, <= 0 selects greedy sampling.
    pub temp: f32,
    /// RNG seed for the final distribution sampler.
    pub seed: u32,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            top_k: 40,
            top_p: 0.95,
            min_p: 0.05,
            temp: 0.8,
            seed: u32::MAX,
        }
    }
}

/// Version string reported by the linked llama.cpp library.
pub fn version() -> &'static str {
    imp::version()
}

/// Last shim error for this thread. Empty when the last call succeeded.
pub fn last_error() -> String {
    imp::last_error()
}

/// Initialise llama.cpp/ggml backends. Optional and idempotent.
pub fn backend_init() {
    imp::backend_init();
}

/// Release global llama.cpp/ggml state. Call once at shutdown, after all
/// [`Model`] and [`Context`] values are dropped.
pub fn backend_free() {
    imp::backend_free();
}

/// Apply a chat template to `messages` (`(role, content)` pairs).
///
/// `template = None` uses the built-in ChatML template. Pass
/// [`Model::chat_template`] to use the model's own template. Returns `Ok(None)`
/// from the template lookup when the file has no template.
pub fn apply_chat_template(
    template: Option<&str>,
    messages: &[(String, String)],
    add_assistant: bool,
) -> Result<String, Error> {
    imp::apply_chat_template(template, messages, add_assistant)
}
