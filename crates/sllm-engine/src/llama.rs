//! Real llama.cpp backend.
//!
//! The backend lives behind the `llama` feature so the default build and host
//! tests do not need CMake, MSVC or the pinned llama.cpp tree. It implements
//! the same [`Engine`] contract as [`crate::MockEngine`]; only the numeric
//! fields (`mock: false`, real token ids and timings) differ.
//!
//! Threading model:
//!
//! - `load`, `trim` and `unload` are serialised by `op` and touch the session
//!   only while no generation is running.
//! - `generate` holds the session mutex for the whole stream. `stats` reads a
//!   separate state mutex, and `cancel` is an atomic flag, so a sink can call
//!   them without deadlocking.
//! - `trim`/`unload` that arrive during generation are recorded as pending and
//!   applied when the stream finishes, exactly like [`crate::MockEngine`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use sllm_core::{Backend, EngineConfig, ErrorCode, KvType, SllmError};
use sllm_llama_sys::{
    Context, ContextOptions, Error as SysError, ErrorKind, GGML_TYPE_F16, GGML_TYPE_Q4_0,
    GGML_TYPE_Q8_0, Model, ModelOptions, Sampler, SamplerConfig as SysSamplerConfig,
    apply_chat_template,
};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::stop::StopBuffer;
use crate::types::{
    EngineEvent, EngineState, EngineStats, GenStats, GenerateRequest, LoraRef, Message, ModelRef,
    TrimLevel,
};

/// Repetition-penalty window used when `repeat_penalty != 1.0`.
const REPEAT_LAST_N: i32 = 64;

static BACKEND_INIT: Once = Once::new();

fn ensure_backend() {
    BACKEND_INIT.call_once(sllm_llama_sys::backend_init);
}

fn lock_state(state: &Mutex<LlamaState>) -> MutexGuard<'_, LlamaState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_session(session: &Mutex<Option<Session>>) -> MutexGuard<'_, Option<Session>> {
    session
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_op(op: &Mutex<()>) -> MutexGuard<'_, ()> {
    op.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug)]
struct LlamaState {
    model: Option<ModelRef>,
    status: EngineState,
    trim: TrimLevel,
    /// Mirrors `session.is_some()`. Kept in the state so a panicking sink can
    /// restore the status without locking the session.
    weights: bool,
    requests: u64,
    generated_tokens: u64,
    last: Option<GenStats>,
    pending_trim: TrimLevel,
    pending_unload: bool,
}

impl Default for LlamaState {
    fn default() -> Self {
        Self {
            model: None,
            status: EngineState::Unloaded,
            trim: TrimLevel::None,
            weights: false,
            requests: 0,
            generated_tokens: 0,
            last: None,
            pending_trim: TrimLevel::None,
            pending_unload: false,
        }
    }
}

impl LlamaState {
    fn status_for(&self) -> EngineState {
        if self.model.is_none() {
            EngineState::Unloaded
        } else if self.weights {
            EngineState::Ready
        } else {
            EngineState::Trimmed
        }
    }
}

/// Model, context and template owned by the engine.
///
/// `ctx` must drop before `model`, so it is declared first. The context was
/// created from a `Box<Model>` allocation that the session keeps alive; moving
/// the session moves the box pointer, not the model, so the lifetime extension
/// below stays valid.
struct Session {
    ctx: Option<Context<'static>>,
    model: Box<Model>,
    ctx_options: ContextOptions,
    chat_template: Option<String>,
    n_batch: u32,
    loras: Vec<LoraRef>,
}

impl Session {
    fn open(model: &ModelRef, config: &EngineConfig) -> Result<Self, EngineError> {
        let load_mode = match (config.mmap, config.mlock) {
            (true, true) => sllm_llama_sys::LOAD_MODE_MMAP_MLOCK,
            (true, false) => sllm_llama_sys::LOAD_MODE_MMAP,
            (false, true) => sllm_llama_sys::LOAD_MODE_MLOCK,
            (false, false) => sllm_llama_sys::LOAD_MODE_NONE,
        };
        let model_options = ModelOptions {
            n_gpu_layers: 0,
            load_mode,
        };
        let ctx_options = ContextOptions {
            n_ctx: config.ctx,
            n_batch: config.batch,
            n_ubatch: config.batch.min(512),
            n_threads: i32::try_from(config.threads)
                .map_err(|_| EngineError::config("threads do not fit in i32"))?,
            n_threads_batch: i32::try_from(config.threads)
                .map_err(|_| EngineError::config("threads do not fit in i32"))?,
            type_k: kv_type(config.kv),
            type_v: kv_type(config.kv),
            offload_kqv: false,
            flash_attn_type: if config.flash_attn { 1 } else { 0 },
        };

        let model_loras = model.loras.clone();
        let model = Model::load_with(&model.path, model_options)
            .map_err(|error| map_load("open model", error))?;
        let chat_template = model.chat_template();
        let model = Box::new(model);

        // SAFETY: the model is boxed, never moved out of the session, and the
        // context field is declared before the model so it is dropped first.
        let model_ref: &'static Model = unsafe { &*(&*model as *const Model) };
        let mut ctx = model_ref
            .context(&ctx_options)
            .map_err(|error| map_load("create context", error))?;
        for lora in &model_loras {
            ctx.apply_lora(&lora.path, lora.scale)
                .map_err(|error| map_load("apply LoRA", error))?;
        }

        Ok(Self {
            ctx: Some(ctx),
            model,
            ctx_options,
            chat_template,
            n_batch: config.batch,
            loras: model_loras,
        })
    }

    /// Recreate the context after `trim(RunningLow)`.
    fn context(&mut self) -> Result<&mut Context<'static>, EngineError> {
        if self.ctx.is_none() {
            // SAFETY: same invariant as `Session::open`.
            let model: &'static Model = unsafe { &*(&*self.model as *const Model) };
            let mut ctx = model
                .context(&self.ctx_options)
                .map_err(|error| map_load("recreate context", error))?;
            for lora in &self.loras {
                ctx.apply_lora(&lora.path, lora.scale)
                    .map_err(|error| map_load("apply LoRA", error))?;
            }
            self.ctx = Some(ctx);
        }
        self.ctx
            .as_mut()
            .ok_or_else(|| EngineError::load("context is missing"))
    }

    fn drop_context(&mut self) {
        self.ctx = None;
    }

    /// Drop stale KV state before an independent request.
    fn reset_memory(&mut self) -> Result<(), EngineError> {
        self.context()?.clear_memory();
        Ok(())
    }
}

fn kv_type(kv: KvType) -> i32 {
    match kv {
        KvType::F16 => GGML_TYPE_F16,
        KvType::Q4_0 => GGML_TYPE_Q4_0,
        KvType::Q8_0 => GGML_TYPE_Q8_0,
    }
}

fn map_load(stage: &str, error: SysError) -> EngineError {
    let message = format!("{stage}: {error}");
    match error.kind() {
        ErrorKind::OutOfMemory => EngineError::from(SllmError::new(ErrorCode::RamLow, message)),
        ErrorKind::InvalidArgument => EngineError::config(message),
        _ => EngineError::load(message),
    }
}

fn map_generate(stage: &str, error: SysError) -> EngineError {
    EngineError::generate(format!("{stage}: {error}"))
}

/// Concrete [`Engine`] backed by the pinned llama.cpp fork.
pub struct LlamaEngine {
    state: Mutex<LlamaState>,
    /// Control-plane lock. Never held while streaming.
    op: Mutex<()>,
    session: Mutex<Option<Session>>,
    cancel: AtomicBool,
    busy: AtomicBool,
}

impl std::fmt::Debug for LlamaEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = lock_state(&self.state);
        f.debug_struct("LlamaEngine")
            .field("status", &state.status)
            .field("model", &state.model.as_ref().map(|model| model.id.clone()))
            .field("busy", &self.busy.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl Default for LlamaEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LlamaEngine {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(LlamaState::default()),
            op: Mutex::new(()),
            session: Mutex::new(None),
            cancel: AtomicBool::new(false),
            busy: AtomicBool::new(false),
        }
    }
}

impl Engine for LlamaEngine {
    fn load(&self, model: &ModelRef, config: &EngineConfig) -> Result<(), EngineError> {
        let _op = lock_op(&self.op);
        if self.busy.load(Ordering::Acquire) {
            return Err(EngineError::busy());
        }

        validate_model(model)?;
        config.validate().map_err(EngineError::from)?;
        validate_config(config)?;

        ensure_backend();
        let session = Session::open(model, config)?;

        {
            let mut slot = lock_session(&self.session);
            *slot = Some(session);
        }
        let mut state = lock_state(&self.state);
        state.model = Some(model.clone());
        state.status = EngineState::Ready;
        state.trim = TrimLevel::None;
        state.weights = true;
        state.requests = 0;
        state.generated_tokens = 0;
        state.last = None;
        state.pending_trim = TrimLevel::None;
        state.pending_unload = false;
        self.cancel.store(false, Ordering::Release);
        Ok(())
    }

    fn generate(
        &self,
        request: &GenerateRequest,
        sink: &mut dyn FnMut(EngineEvent),
    ) -> Result<GenStats, EngineError> {
        let started = Instant::now();

        {
            let _op = lock_op(&self.op);
            if self.busy.load(Ordering::Acquire) {
                return Err(EngineError::busy());
            }
            self.cancel.store(false, Ordering::Release);
            self.busy.store(true, Ordering::Release);
        }
        let _busy = BusyFlag(&self.busy);

        if let Err(error) = request.validate() {
            return emit_error(sink, error);
        }
        if request.messages.is_empty() {
            return emit_error(sink, EngineError::config("messages are empty"));
        }

        let mut session_guard = lock_session(&self.session);
        let Some(session) = session_guard.as_mut() else {
            let state = lock_state(&self.state);
            let message = if state.model.is_some() {
                "model weights were released by trim(complete); load again"
            } else {
                "no model loaded; call load first"
            };
            drop(state);
            return emit_error(sink, EngineError::no_model(message));
        };

        if let Err(error) = session.reset_memory() {
            return emit_error(sink, error);
        }

        let tokens = match build_tokens(session, &request.messages) {
            Ok(tokens) => tokens,
            Err(error) => return emit_error(sink, error),
        };
        if tokens.is_empty() {
            return emit_error(sink, EngineError::config("the prompt produced no tokens"));
        }
        let n_ctx = session
            .ctx
            .as_ref()
            .map(|ctx| ctx.n_ctx() as usize)
            .unwrap_or(0);
        if tokens.len() >= n_ctx {
            return emit_error(
                sink,
                EngineError::config(format!(
                    "prompt is {} tokens but context is {n_ctx}; shorten the prompt",
                    tokens.len()
                )),
            );
        }
        let max_new = (request.max_tokens as usize).min(n_ctx - tokens.len());

        {
            let mut state = lock_state(&self.state);
            state.status = EngineState::Generating;
            state.requests += 1;
        }
        let mut restore = RestoreState {
            state: &self.state,
            armed: true,
        };

        sink(EngineEvent::Started {
            prompt_tokens: tokens.len(),
        });

        let outcome = stream(
            session,
            request,
            &tokens,
            max_new,
            started,
            &self.cancel,
            sink,
        );
        let stats = match outcome {
            Ok(stats) => stats,
            Err(error) => return emit_error(sink, error),
        };

        {
            let mut state = lock_state(&self.state);
            state.generated_tokens += stats.generated_tokens as u64;
            state.last = Some(stats.clone());
            if state.pending_unload {
                *session_guard = None;
                apply_unload(&mut state);
            } else {
                let pending = std::mem::take(&mut state.pending_trim);
                if pending != TrimLevel::None {
                    apply_trim(&mut state, &mut session_guard, pending);
                }
            }
            state.status = state.status_for();
        }
        restore.armed = false;

        sink(EngineEvent::Done {
            stats: stats.clone(),
        });
        Ok(stats)
    }

    fn cancel(&self) {
        if self.busy.load(Ordering::Acquire) {
            self.cancel.store(true, Ordering::Release);
        }
    }

    fn trim(&self, level: TrimLevel) {
        let _op = lock_op(&self.op);
        if self.busy.load(Ordering::Acquire) {
            let mut state = lock_state(&self.state);
            state.pending_trim = state.pending_trim.max(level);
            return;
        }
        let mut session = lock_session(&self.session);
        let mut state = lock_state(&self.state);
        apply_trim(&mut state, &mut session, level);
    }

    fn stats(&self) -> EngineStats {
        let state = lock_state(&self.state);
        EngineStats {
            state: state.status,
            model_id: state.model.as_ref().map(|model| model.id.clone()),
            trim: state.trim,
            needs_reload: state.model.is_some() && !state.weights,
            requests: state.requests,
            generated_tokens: state.generated_tokens,
            last: state.last.clone(),
            mock: false,
        }
    }

    fn unload(&self) {
        let _op = lock_op(&self.op);
        if self.busy.load(Ordering::Acquire) {
            let mut state = lock_state(&self.state);
            state.pending_unload = true;
            return;
        }
        {
            let mut session = lock_session(&self.session);
            *session = None;
        }
        let mut state = lock_state(&self.state);
        apply_unload(&mut state);
    }
}

fn validate_model(model: &ModelRef) -> Result<(), EngineError> {
    if model.id.trim().is_empty() {
        return Err(EngineError::config("model id is empty"));
    }
    if model.path.as_os_str().is_empty() {
        return Err(EngineError::config("model path is empty"));
    }
    match std::fs::metadata(&model.path) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Err(EngineError::io(format!(
                "{} is not a file",
                model.path.display()
            )));
        }
        Err(error) => {
            return Err(EngineError::io(format!(
                "cannot open {}: {error}",
                model.path.display()
            )));
        }
    }
    for lora in &model.loras {
        if lora.path.as_os_str().is_empty() {
            return Err(EngineError::config("LoRA path is empty"));
        }
        if !lora.path.is_file() {
            return Err(EngineError::io(format!(
                "LoRA adapter does not exist: {}",
                lora.path.display()
            )));
        }
        if !lora.scale.is_finite() || lora.scale <= 0.0 {
            return Err(EngineError::config("LoRA scale must be finite and > 0"));
        }
    }
    if model.mmproj.is_some() {
        return Err(EngineError::config(
            "mmproj/vision is not exposed by this llama.cpp backend yet",
        ));
    }
    Ok(())
}

fn validate_config(config: &EngineConfig) -> Result<(), EngineError> {
    match config.backend {
        Backend::Auto | Backend::Cpu => {}
        other => {
            return Err(EngineError::config(format!(
                "backend {other:?} is not available in this build"
            )));
        }
    }
    if config.gpu_layers > 0 {
        return Err(EngineError::config(
            "gpu_layers > 0 needs a GPU backend; this build is CPU-only",
        ));
    }
    if config.expert_cache_mb > 0 {
        return Err(EngineError::config(
            "expert_cache_mb is not available on the CPU backend",
        ));
    }
    Ok(())
}

fn build_tokens(session: &Session, messages: &[Message]) -> Result<Vec<i32>, EngineError> {
    let pairs: Vec<(String, String)> = messages
        .iter()
        .map(|message| (message.role.as_str().to_string(), message.content.clone()))
        .collect();

    let prompt = match session.chat_template.as_deref() {
        Some(template) if !template.trim().is_empty() => {
            match apply_chat_template(Some(template), &pairs, true) {
                Ok(prompt) => prompt,
                // The legacy template engine knows a fixed list. Fall back to
                // ChatML rather than failing a model with an exotic template.
                Err(_) => apply_chat_template(None, &pairs, true).map_err(|error| {
                    EngineError::config(format!("chat template failed: {error}"))
                })?,
            }
        }
        _ => apply_chat_template(None, &pairs, true)
            .map_err(|error| EngineError::config(format!("chat template failed: {error}")))?,
    };

    let ctx = session
        .ctx
        .as_ref()
        .ok_or_else(|| EngineError::no_model("context is not available"))?;
    let mut tokens = ctx
        .tokenize(&prompt, false, true)
        .map_err(|error| EngineError::config(format!("tokenize failed: {error}")))?;

    if ctx.add_bos() {
        let bos = ctx.bos_token();
        if bos >= 0 && tokens.first().copied() != Some(bos) {
            tokens.insert(0, bos);
        }
    }
    Ok(tokens)
}

fn decode_prompt(session: &mut Session, tokens: &[i32]) -> Result<(), EngineError> {
    let n_batch = session.n_batch.max(1) as usize;
    let ctx = session
        .ctx
        .as_mut()
        .ok_or_else(|| EngineError::no_model("context is not available"))?;
    let mut pos = 0i32;
    for chunk in tokens.chunks(n_batch) {
        ctx.decode(chunk, pos, 0, false)
            .map_err(|error| map_generate("prefill decode", error))?;
        pos += chunk.len() as i32;
    }
    Ok(())
}

fn stream(
    session: &mut Session,
    request: &GenerateRequest,
    tokens: &[i32],
    max_new: usize,
    started: Instant,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(EngineEvent),
) -> Result<GenStats, EngineError> {
    let mut canceled = false;
    let mut prefill_ms = 0u64;

    if max_new > 0 && cancel.load(Ordering::Acquire) {
        canceled = true;
    }
    if max_new > 0 && !canceled {
        decode_prompt(session, tokens)?;
        prefill_ms = started.elapsed().as_millis() as u64;
    }

    let sys_sampler = SysSamplerConfig {
        top_k: i32::try_from(request.sampler.top_k).unwrap_or(i32::MAX),
        top_p: request.sampler.top_p,
        min_p: request.sampler.min_p,
        temp: request.sampler.temperature,
        seed: if request.seed < 0 {
            random_seed()
        } else {
            request.seed as u64 as u32
        },
    };

    let mut generated = 0usize;
    let mut first_token_at = None;
    let mut last_id = 0u32;
    let mut text = TokenText::new();
    let mut stops = StopBuffer::new(&request.stop);

    if max_new > 0 && !canceled {
        let n_vocab = session.ctx.as_ref().map(|ctx| ctx.n_vocab()).unwrap_or(0);
        let mut sampler = Sampler::new_with_penalty(
            sys_sampler,
            n_vocab,
            request.sampler.repeat_penalty,
            REPEAT_LAST_N,
        )
        .map_err(|error| map_generate("sampler init", error))?;

        let n_ctx = session.ctx.as_ref().map(|ctx| ctx.n_ctx()).unwrap_or(0) as i32;
        let mut pos = tokens.len() as i32;

        loop {
            if generated >= max_new {
                break;
            }
            if cancel.load(Ordering::Acquire) {
                canceled = true;
                break;
            }
            if pos >= n_ctx {
                break;
            }

            let ctx = session
                .ctx
                .as_mut()
                .ok_or_else(|| EngineError::no_model("context is not available"))?;

            let token = sampler
                .sample(ctx)
                .map_err(|error| map_generate("sample", error))?;
            if ctx.is_eog(token) {
                break;
            }

            if first_token_at.is_none() {
                first_token_at = Some(Instant::now());
            }
            generated += 1;
            last_id = token as u32;

            let piece = ctx
                .token_to_piece_bytes(token)
                .map_err(|error| map_generate("detokenize", error))?;
            let piece = text.push(&piece);
            if !piece.is_empty() {
                let visible = stops.push(&piece);
                if !visible.is_empty() {
                    sink(EngineEvent::Token {
                        text: visible,
                        id: last_id,
                    });
                }
            }
            if stops.hit() || generated >= max_new {
                break;
            }

            ctx.decode(&[token], pos, 0, false)
                .map_err(|error| map_generate("decode", error))?;
            pos += 1;
        }
    }

    // A stop string hit drops the held tail. Cancellation drops it too, as the
    // contract says; a normal end flushes it.
    if !canceled && !stops.hit() && generated > 0 {
        let tail = stops.finish();
        if !tail.is_empty() {
            sink(EngineEvent::Token {
                text: tail,
                id: last_id,
            });
        }
    }

    let finished = Instant::now();
    let total_ms = finished.duration_since(started).as_millis() as u64;
    let ttft_ms = first_token_at
        .map(|at| at.duration_since(started).as_millis() as u64)
        .unwrap_or(0);
    let decode_ms = first_token_at
        .map(|at| finished.duration_since(at).as_millis() as u64)
        .unwrap_or(0);
    let tok_per_s = if decode_ms > 0 {
        (generated.saturating_sub(1) as f64 * 1000.0 / decode_ms as f64) as f32
    } else if total_ms > 0 {
        (generated as f64 * 1000.0 / total_ms as f64) as f32
    } else {
        0.0
    };

    Ok(GenStats {
        prompt_tokens: tokens.len(),
        generated_tokens: generated,
        ttft_ms,
        prefill_ms,
        decode_ms,
        tok_per_s,
        seed: request.seed,
        canceled,
        mock: false,
    })
}

fn random_seed() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let nanos = nanos as u64;
    (nanos ^ (nanos >> 32)) as u32
}

fn apply_trim(state: &mut LlamaState, session: &mut Option<Session>, level: TrimLevel) {
    if state.model.is_none() {
        return;
    }
    let effective = state.trim.max(level);
    state.trim = effective;
    if effective >= TrimLevel::Complete {
        *session = None;
        state.weights = false;
    } else {
        if effective >= TrimLevel::RunningLow
            && let Some(session) = session.as_mut()
        {
            // Drop the KV cache; the weights stay mapped and the next
            // generation recreates the context from the model.
            session.drop_context();
        }
        state.weights = session.is_some();
    }
    state.status = state.status_for();
}

fn apply_unload(state: &mut LlamaState) {
    state.model = None;
    state.status = EngineState::Unloaded;
    state.trim = TrimLevel::None;
    state.weights = false;
    state.pending_trim = TrimLevel::None;
    state.pending_unload = false;
}

fn emit_error(
    sink: &mut dyn FnMut(EngineEvent),
    error: EngineError,
) -> Result<GenStats, EngineError> {
    sink(EngineEvent::from_error(&error));
    Err(error)
}

/// Clears the busy flag even if a sink panics.
struct BusyFlag<'a>(&'a AtomicBool);

impl Drop for BusyFlag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Restores `Ready`/`Trimmed` if the stream unwinds through a panicking sink.
struct RestoreState<'a> {
    state: &'a Mutex<LlamaState>,
    armed: bool,
}

impl Drop for RestoreState<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut state = lock_state(self.state);
        state.status = state.status_for();
    }
}

/// Buffers raw token bytes so a UTF-8 character split across tokens is emitted
/// whole.
struct TokenText {
    pending: Vec<u8>,
}

impl TokenText {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.pending) {
            Ok(_) => self.pending.len(),
            Err(error) => error.valid_up_to(),
        };
        if valid == 0 {
            return String::new();
        }
        let text = std::str::from_utf8(&self.pending[..valid])
            .map(str::to_string)
            .unwrap_or_default();
        self.pending.drain(..valid);
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kv_types_match_ggml() {
        assert_eq!(kv_type(KvType::F16), 1);
        assert_eq!(kv_type(KvType::Q4_0), 2);
        assert_eq!(kv_type(KvType::Q8_0), 8);
    }

    #[test]
    fn token_text_keeps_split_utf8_whole() {
        let bytes = "Привет".as_bytes();
        let mut text = TokenText::new();
        assert_eq!(text.push(&bytes[..1]), "");
        let rest = text.push(&bytes[1..3]);
        assert_eq!(rest, "П");
        let rest = text.push(&bytes[3..]);
        assert_eq!(rest, "ривет");
    }

    #[test]
    fn engine_is_send_sync_and_boxable_behind_arc() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<LlamaEngine>();

        let engine: std::sync::Arc<dyn Engine> = std::sync::Arc::new(LlamaEngine::new());
        let stats = engine.stats();
        assert_eq!(stats.state, EngineState::Unloaded);
        assert!(!stats.mock);
    }
}
