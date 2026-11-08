use crate::{ContextOptions, Error, ModelOptions, SamplerConfig, raw};
use std::ffi::{CStr, CString, c_char};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr;

fn last_message() -> String {
    unsafe {
        let p = raw::sllm_last_error();
        if p.is_null() {
            String::new()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

fn error(code: i32) -> Error {
    Error::new(code, last_message())
}

fn check(code: i32) -> Result<(), Error> {
    if code == 0 { Ok(()) } else { Err(error(code)) }
}

fn cstring(path: &Path) -> Result<CString, Error> {
    CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| Error::invalid_argument("path contains an interior NUL byte"))
}

pub fn version() -> &'static str {
    unsafe {
        let p = raw::sllm_version();
        if p.is_null() {
            return "unknown";
        }
        CStr::from_ptr(p).to_str().unwrap_or("unknown")
    }
}

pub fn last_error() -> String {
    last_message()
}

pub fn backend_init() {
    unsafe { raw::sllm_backend_init() }
}

pub fn backend_free() {
    unsafe { raw::sllm_backend_free() }
}

pub fn apply_chat_template(
    template: Option<&str>,
    messages: &[(String, String)],
    add_assistant: bool,
) -> Result<String, Error> {
    let template = match template {
        Some(text) => Some(
            CString::new(text.as_bytes())
                .map_err(|_| Error::invalid_argument("chat template contains a NUL byte"))?,
        ),
        None => None,
    };

    let mut roles = Vec::with_capacity(messages.len());
    let mut contents = Vec::with_capacity(messages.len());
    for (role, content) in messages {
        roles.push(
            CString::new(role.as_bytes())
                .map_err(|_| Error::invalid_argument("message role contains a NUL byte"))?,
        );
        contents.push(
            CString::new(content.as_bytes())
                .map_err(|_| Error::invalid_argument("message content contains a NUL byte"))?,
        );
    }

    let role_ptrs: Vec<*const c_char> = roles.iter().map(|s| s.as_ptr()).collect();
    let content_ptrs: Vec<*const c_char> = contents.iter().map(|s| s.as_ptr()).collect();
    let n_msg =
        i32::try_from(messages.len()).map_err(|_| Error::invalid_argument("too many messages"))?;
    let template_ptr = template.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());

    let mut buf = vec![0u8; 256];
    loop {
        let rc = unsafe {
            raw::sllm_chat_apply_template(
                template_ptr,
                role_ptrs.as_ptr(),
                content_ptrs.as_ptr(),
                n_msg,
                add_assistant,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as i32,
            )
        };
        if rc < 0 {
            return Err(error(rc));
        }
        let needed = rc as usize;
        if needed < buf.len() {
            buf.truncate(needed);
            return Ok(String::from_utf8_lossy(&buf).into_owned());
        }
        buf.resize(needed + 1, 0);
    }
}

impl Default for ModelOptions {
    fn default() -> Self {
        let mut opts = raw::sllm_model_options {
            n_gpu_layers: 0,
            load_mode: crate::LOAD_MODE_MMAP,
        };
        let rc = unsafe { raw::sllm_model_default_options(&mut opts) };
        if rc != 0 {
            return Self {
                n_gpu_layers: 0,
                load_mode: crate::LOAD_MODE_MMAP,
            };
        }
        Self {
            n_gpu_layers: opts.n_gpu_layers,
            load_mode: opts.load_mode,
        }
    }
}

impl ModelOptions {
    fn to_raw(self) -> raw::sllm_model_options {
        raw::sllm_model_options {
            n_gpu_layers: self.n_gpu_layers,
            load_mode: self.load_mode,
        }
    }
}

impl Default for ContextOptions {
    fn default() -> Self {
        let mut opts = raw::sllm_context_options {
            n_ctx: 0,
            n_batch: 2048,
            n_ubatch: 512,
            n_threads: 4,
            n_threads_batch: 4,
            type_k: 1,
            type_v: 1,
            offload_kqv: true,
            flash_attn_type: -1,
        };
        let rc = unsafe { raw::sllm_context_default_options(&mut opts) };
        if rc != 0 {
            return Self {
                n_ctx: 0,
                n_batch: 2048,
                n_ubatch: 512,
                n_threads: 4,
                n_threads_batch: 4,
                type_k: 1,
                type_v: 1,
                offload_kqv: true,
                flash_attn_type: -1,
            };
        }
        Self {
            n_ctx: opts.n_ctx,
            n_batch: opts.n_batch,
            n_ubatch: opts.n_ubatch,
            n_threads: opts.n_threads,
            n_threads_batch: opts.n_threads_batch,
            type_k: opts.type_k,
            type_v: opts.type_v,
            offload_kqv: opts.offload_kqv,
            flash_attn_type: opts.flash_attn_type,
        }
    }
}

impl ContextOptions {
    fn to_raw(self) -> raw::sllm_context_options {
        raw::sllm_context_options {
            n_ctx: self.n_ctx,
            n_batch: self.n_batch,
            n_ubatch: self.n_ubatch,
            n_threads: self.n_threads,
            n_threads_batch: self.n_threads_batch,
            type_k: self.type_k,
            type_v: self.type_v,
            offload_kqv: self.offload_kqv,
            flash_attn_type: self.flash_attn_type,
        }
    }
}

/// Loaded llama.cpp model. Dropping it invalidates every borrowed [`Context`].
pub struct Model {
    ptr: *mut raw::sllm_model,
}

unsafe impl Send for Model {}
unsafe impl Sync for Model {}

impl Model {
    /// Load a GGUF model with llama.cpp defaults.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::load_with(path, ModelOptions::default())
    }

    /// Load a GGUF model with explicit options.
    pub fn load_with(path: impl AsRef<Path>, options: ModelOptions) -> Result<Self, Error> {
        let path = cstring(path.as_ref())?;
        let opts = options.to_raw();
        let mut out: *mut raw::sllm_model = ptr::null_mut();
        let rc = unsafe { raw::sllm_model_load(path.as_ptr(), &opts, &mut out) };
        if rc != 0 || out.is_null() {
            return Err(error(rc));
        }
        Ok(Self { ptr: out })
    }

    /// Human-readable model architecture/quantisation description.
    pub fn description(&self) -> String {
        let mut buf = vec![0u8; 256];
        loop {
            let rc = unsafe {
                raw::sllm_model_desc(self.ptr, buf.as_mut_ptr() as *mut c_char, buf.len() as i32)
            };
            if rc < 0 {
                return String::new();
            }
            if (rc as usize) < buf.len() {
                buf.truncate(rc as usize);
                return String::from_utf8_lossy(&buf).into_owned();
            }
            buf.resize(rc as usize + 1, 0);
        }
    }

    /// Chat template stored in the GGUF, if any.
    pub fn chat_template(&self) -> Option<String> {
        unsafe {
            let ptr = raw::sllm_model_chat_template(self.ptr);
            if ptr.is_null() {
                None
            } else {
                Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
            }
        }
    }

    /// Context size the model was trained with.
    pub fn n_ctx_train(&self) -> i32 {
        unsafe { raw::sllm_model_n_ctx_train(self.ptr) }
    }

    /// Create an inference context borrowing this model.
    pub fn context(&self, options: &ContextOptions) -> Result<Context<'_>, Error> {
        let opts = options.to_raw();
        let mut out: *mut raw::sllm_context = ptr::null_mut();
        let rc = unsafe { raw::sllm_context_new(self.ptr, &opts, &mut out) };
        if rc != 0 || out.is_null() {
            return Err(error(rc));
        }
        Ok(Context {
            ptr: out,
            _model: PhantomData,
        })
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { raw::sllm_model_free(self.ptr) };
            self.ptr = ptr::null_mut();
        }
    }
}

/// Inference context. Borrows the [`Model`] it was created from.
pub struct Context<'a> {
    ptr: *mut raw::sllm_context,
    _model: PhantomData<&'a Model>,
}

unsafe impl Send for Context<'_> {}

impl Context<'_> {
    /// Actual context size chosen by llama.cpp.
    pub fn n_ctx(&self) -> u32 {
        unsafe { raw::sllm_context_n_ctx(self.ptr) }
    }

    /// Vocabulary size.
    pub fn n_vocab(&self) -> i32 {
        unsafe { raw::sllm_context_n_vocab(self.ptr) }
    }

    /// `true` when the token ends generation (EOS, EOT, ...).
    pub fn is_eog(&self, token: i32) -> bool {
        unsafe { raw::sllm_vocab_is_eog(self.ptr, token) }
    }

    /// End-of-sequence token id, or a negative value when the vocab has none.
    pub fn eos_token(&self) -> i32 {
        unsafe { raw::sllm_vocab_eos(self.ptr) }
    }

    /// Beginning-of-sequence token id, or a negative value when the vocab has
    /// none.
    pub fn bos_token(&self) -> i32 {
        unsafe { raw::sllm_vocab_bos(self.ptr) }
    }

    /// Whether the tokenizer wants a BOS token prepended to raw text.
    pub fn add_bos(&self) -> bool {
        unsafe { raw::sllm_vocab_get_add_bos(self.ptr) }
    }

    /// Drop KV cache and recurrent state, keeping the context allocated.
    pub fn clear_memory(&mut self) {
        unsafe { raw::sllm_context_clear_memory(self.ptr) }
    }

    /// Load a LoRA adapter from `path` and attach it to this context.
    ///
    /// The adapter is applied at inference time; base weights are not modified.
    /// Adapters stay active until the context is dropped.
    pub fn apply_lora(&mut self, path: impl AsRef<Path>, scale: f32) -> Result<(), Error> {
        let path = cstring(path.as_ref())?;
        let rc = unsafe { raw::sllm_context_apply_lora(self.ptr, path.as_ptr(), scale) };
        check(rc)
    }

    /// Tokenize UTF-8 text. Special tokens are controlled by the two flags.
    pub fn tokenize(
        &self,
        text: &str,
        add_special: bool,
        parse_special: bool,
    ) -> Result<Vec<i32>, Error> {
        let len = i32::try_from(text.len())
            .map_err(|_| Error::invalid_argument("text is larger than i32::MAX"))?;
        let mut tokens = vec![0i32; 32];

        loop {
            let rc = unsafe {
                raw::sllm_tokenize(
                    self.ptr,
                    text.as_ptr() as *const c_char,
                    len,
                    tokens.as_mut_ptr(),
                    tokens.len() as i32,
                    add_special,
                    parse_special,
                )
            };

            if rc >= 0 {
                tokens.truncate(rc as usize);
                return Ok(tokens);
            }

            let needed = (-(rc as i64)) as usize;
            if needed == 0 || needed > 1 << 24 {
                return Err(error(rc));
            }
            tokens.resize(needed, 0);
        }
    }

    /// Raw bytes of one token piece. The piece may split a UTF-8 sequence.
    pub fn token_to_piece_bytes(&self, token: i32) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; 32];
        loop {
            let rc = unsafe {
                raw::sllm_token_to_piece(
                    self.ptr,
                    token,
                    buf.as_mut_ptr() as *mut c_char,
                    buf.len() as i32,
                    0,
                    false,
                )
            };
            if rc >= 0 {
                buf.truncate(rc as usize);
                return Ok(buf);
            }
            let needed = (-(rc as i64)) as usize;
            if needed == 0 || needed > 1 << 24 {
                return Err(error(rc));
            }
            buf.resize(needed, 0);
        }
    }

    /// One token as lossy UTF-8. Use [`Self::token_to_piece_bytes`] for exact
    /// bytes when streaming multi-byte characters.
    pub fn token_to_piece(&self, token: i32) -> Result<String, Error> {
        self.token_to_piece_bytes(token)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Convert tokens back to UTF-8 text.
    pub fn detokenize(&self, tokens: &[i32]) -> Result<String, Error> {
        let n =
            i32::try_from(tokens.len()).map_err(|_| Error::invalid_argument("too many tokens"))?;
        let mut buf = vec![0u8; 256];
        loop {
            let rc = unsafe {
                raw::sllm_detokenize(
                    self.ptr,
                    tokens.as_ptr(),
                    n,
                    buf.as_mut_ptr() as *mut c_char,
                    buf.len() as i32,
                    false,
                    false,
                )
            };
            if rc >= 0 {
                buf.truncate(rc as usize);
                return Ok(String::from_utf8_lossy(&buf).into_owned());
            }
            let needed = (-(rc as i64)) as usize;
            if needed == 0 || needed > 1 << 24 {
                return Err(error(rc));
            }
            buf.resize(needed, 0);
        }
    }

    /// Decode a batch of tokens for sequence `seq_id`.
    ///
    /// `start_pos < 0` lets llama.cpp track positions; otherwise positions are
    /// `start_pos .. start_pos + tokens.len()`.
    pub fn decode(
        &mut self,
        tokens: &[i32],
        start_pos: i32,
        seq_id: i32,
        logits_all: bool,
    ) -> Result<(), Error> {
        let n =
            i32::try_from(tokens.len()).map_err(|_| Error::invalid_argument("too many tokens"))?;
        let rc = unsafe {
            raw::sllm_decode(self.ptr, tokens.as_ptr(), n, start_pos, seq_id, logits_all)
        };
        check(rc)
    }

    /// Logits for the last decoded token. Empty before the first decode.
    ///
    /// The slice is invalidated by the next [`Self::decode`] call.
    pub fn logits(&self) -> &[f32] {
        let n = self.n_vocab();
        if n <= 0 {
            return &[];
        }
        let ptr = unsafe { raw::sllm_get_logits(self.ptr) };
        if ptr.is_null() {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(ptr, n as usize) }
        }
    }
}

impl Drop for Context<'_> {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { raw::sllm_context_free(self.ptr) };
            self.ptr = ptr::null_mut();
        }
    }
}

/// Sampler chain owned by Rust.
pub struct Sampler {
    ptr: *mut raw::sllm_sampler,
}

unsafe impl Send for Sampler {}

impl Sampler {
    pub fn new(config: SamplerConfig) -> Result<Self, Error> {
        let ptr = unsafe {
            raw::sllm_sampler_new(
                config.top_k,
                config.top_p,
                config.min_p,
                config.temp,
                config.seed,
            )
        };
        if ptr.is_null() {
            return Err(error(-4));
        }
        Ok(Self { ptr })
    }

    /// Build a sampler chain with a repetition-penalty stage.
    ///
    /// `n_vocab` comes from [`Context::n_vocab`]; `repeat_last_n == 0` or
    /// `repeat_penalty == 1.0` disables the stage.
    pub fn new_with_penalty(
        config: SamplerConfig,
        n_vocab: i32,
        repeat_penalty: f32,
        repeat_last_n: i32,
    ) -> Result<Self, Error> {
        let ptr = unsafe {
            raw::sllm_sampler_new_ex(
                config.top_k,
                config.top_p,
                config.min_p,
                config.temp,
                n_vocab,
                repeat_penalty,
                repeat_last_n,
                config.seed,
            )
        };
        if ptr.is_null() {
            return Err(error(-4));
        }
        Ok(Self { ptr })
    }

    /// Sample the next token from the last decode of `ctx`.
    pub fn sample(&mut self, ctx: &Context<'_>) -> Result<i32, Error> {
        let rc = unsafe { raw::sllm_sampler_sample(self.ptr, ctx.ptr) };
        if rc < 0 { Err(error(rc)) } else { Ok(rc) }
    }

    /// Register a token that was selected outside [`Self::sample`] (for
    /// example a forced tool token or a token injected by the caller).
    /// [`Self::sample`] already accepts the token it returns, so calling this
    /// for a sampled token would apply it twice.
    pub fn accept(&mut self, token: i32) {
        unsafe { raw::sllm_sampler_accept(self.ptr, token) }
    }

    pub fn reset(&mut self) {
        unsafe { raw::sllm_sampler_reset(self.ptr) }
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { raw::sllm_sampler_free(self.ptr) };
            self.ptr = ptr::null_mut();
        }
    }
}
