// Stub implementation compiled with SLLM_SYS_SKIP_BUILD=1.
//
// It keeps the public API checkable on machines without a C++ toolchain while
// making it obvious that no inference can happen.

use crate::{ContextOptions, Error, ModelOptions, SamplerConfig};
use std::marker::PhantomData;
use std::path::Path;

pub fn version() -> &'static str {
    "unavailable (built with SLLM_SYS_SKIP_BUILD=1)"
}

pub fn last_error() -> String {
    String::new()
}

pub fn backend_init() {}

pub fn backend_free() {}

pub fn apply_chat_template(
    _template: Option<&str>,
    _messages: &[(String, String)],
    _add_assistant: bool,
) -> Result<String, Error> {
    Err(Error::not_built())
}

impl Default for ModelOptions {
    fn default() -> Self {
        Self {
            n_gpu_layers: 0,
            load_mode: crate::LOAD_MODE_MMAP,
        }
    }
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            n_ctx: 0,
            n_batch: 2048,
            n_ubatch: 512,
            n_threads: 4,
            n_threads_batch: 4,
            type_k: 1,
            type_v: 1,
            offload_kqv: true,
            flash_attn_type: -1,
        }
    }
}

/// Stub model: no native library is linked.
pub struct Model {
    _private: PhantomData<()>,
}

impl Model {
    pub fn load(_path: impl AsRef<Path>) -> Result<Self, Error> {
        Err(Error::not_built())
    }

    pub fn load_with(_path: impl AsRef<Path>, _options: ModelOptions) -> Result<Self, Error> {
        Err(Error::not_built())
    }

    pub fn description(&self) -> String {
        String::new()
    }

    pub fn chat_template(&self) -> Option<String> {
        None
    }

    pub fn n_ctx_train(&self) -> i32 {
        0
    }

    pub fn context(&self, _options: &ContextOptions) -> Result<Context<'_>, Error> {
        Err(Error::not_built())
    }
}

/// Stub context: no native library is linked.
pub struct Context<'a> {
    _model: PhantomData<&'a Model>,
}

impl Context<'_> {
    pub fn n_ctx(&self) -> u32 {
        0
    }

    pub fn n_vocab(&self) -> i32 {
        0
    }

    pub fn is_eog(&self, _token: i32) -> bool {
        false
    }

    pub fn eos_token(&self) -> i32 {
        -1
    }

    pub fn bos_token(&self) -> i32 {
        -1
    }

    pub fn add_bos(&self) -> bool {
        false
    }

    pub fn tokenize(
        &self,
        _text: &str,
        _add_special: bool,
        _parse_special: bool,
    ) -> Result<Vec<i32>, Error> {
        Err(Error::not_built())
    }

    pub fn clear_memory(&mut self) {}

    pub fn apply_lora(
        &mut self,
        _path: impl AsRef<std::path::Path>,
        _scale: f32,
    ) -> Result<(), Error> {
        Err(Error::not_built())
    }

    pub fn token_to_piece_bytes(&self, _token: i32) -> Result<Vec<u8>, Error> {
        Err(Error::not_built())
    }

    pub fn token_to_piece(&self, _token: i32) -> Result<String, Error> {
        Err(Error::not_built())
    }

    pub fn detokenize(&self, _tokens: &[i32]) -> Result<String, Error> {
        Err(Error::not_built())
    }

    pub fn decode(
        &mut self,
        _tokens: &[i32],
        _start_pos: i32,
        _seq_id: i32,
        _logits_all: bool,
    ) -> Result<(), Error> {
        Err(Error::not_built())
    }

    pub fn logits(&self) -> &[f32] {
        &[]
    }
}

/// Stub sampler: no native library is linked.
pub struct Sampler {
    _private: PhantomData<()>,
}

impl Sampler {
    pub fn new(_config: SamplerConfig) -> Result<Self, Error> {
        Err(Error::not_built())
    }

    pub fn new_with_penalty(
        _config: SamplerConfig,
        _n_vocab: i32,
        _repeat_penalty: f32,
        _repeat_last_n: i32,
    ) -> Result<Self, Error> {
        Err(Error::not_built())
    }

    pub fn sample(&mut self, _ctx: &Context<'_>) -> Result<i32, Error> {
        Err(Error::not_built())
    }

    pub fn accept(&mut self, _token: i32) {}

    pub fn reset(&mut self) {}
}
