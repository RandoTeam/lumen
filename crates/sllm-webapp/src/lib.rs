//! Thin JNI bridge for the WebView shell.
//!
//! The UI is Preact in a system WebView. Model work stays in Rust; Java only
//! owns the Activity, the WebView and the JNI method declarations.
//!
//! This crate links the **real** llama.cpp toolchain and the real
//! `sllm-engine::LlamaEngine` into the APK. [`host`] owns the engine and streams
//! generation events to the WebView; [`llama_smoke`] remains as a small
//! diagnostic that proves load/tokenize/prefill/decode independently.
//!
//! There is no mock chat backend. When the bridge is absent the UI says so.

use std::path::Path;
#[cfg(target_os = "android")]
use std::sync::Once;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::jstring;
use serde_json::json;
use sllm_llama_sys::{
    ContextOptions, Model, ModelOptions, Sampler, SamplerConfig, version as llama_version,
};
use sllm_model::ModelRegistry;

mod host;

#[cfg(target_os = "android")]
static INIT: Once = Once::new();

fn init_log() {
    #[cfg(target_os = "android")]
    INIT.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("SLLM"),
        );
    });
}

fn error_json(avail_mb: u64, error: impl std::fmt::Display) -> String {
    json!({ "availMb": avail_mb, "models": [], "error": error.to_string() }).to_string()
}

fn scan(dir: &str, avail_mb: u64) -> String {
    match ModelRegistry::discover(Path::new(dir)) {
        Ok(registry) => match registry.to_json(avail_mb) {
            Ok(json) => json,
            Err(error) => {
                log::warn!("registry json failed: {error}");
                error_json(avail_mb, error)
            }
        },
        Err(error) => {
            log::warn!("scan {dir} failed: {error}");
            error_json(avail_mb, error)
        }
    }
}

// ---------------------------------------------------------------------------
// llama.cpp smoke test
// ---------------------------------------------------------------------------

/// Short prompt used by the smoke test. No chat template is applied: the goal
/// is to prove tokenize -> prefill -> sample -> detokenize on the phone.
const SMOKE_PROMPT: &str = "The capital of France is";
const SMOKE_MAX_TOKENS: usize = 4;
const SMOKE_CONTEXT: u32 = 2048;
const SMOKE_THREADS: i32 = 4;

struct LoadedModel {
    path: String,
    model: Model,
}

/// The model is kept resident after a smoke run so PSS can be measured with
/// Qwen loaded. `nativeLlamaUnload` drops it explicitly.
static MODEL: OnceLock<Mutex<Option<LoadedModel>>> = OnceLock::new();

fn model_slot() -> &'static Mutex<Option<LoadedModel>> {
    MODEL.get_or_init(|| Mutex::new(None))
}

fn smoke_error(stage: &str, error: &sllm_llama_sys::Error, load_ms: u64) -> String {
    log::error!("llama smoke {stage} failed: {error}");
    json!({
        "ok": false,
        "version": llama_version(),
        "platform": sllm_llama_sys::PLATFORM,
        "stage": stage,
        "loadMs": load_ms,
        "prefillMs": 0,
        "decodeMs": 0,
        "tokens": [],
        "tokenCount": 0,
        "text": "",
        "error": error.to_string(),
        "lastError": sllm_llama_sys::last_error(),
    })
    .to_string()
}

fn llama_smoke(model_path: &str) -> String {
    log::info!(
        "llama smoke: version={} model={model_path}",
        llama_version()
    );
    let started = Instant::now();

    let mut slot = model_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let mut load_ms = 0u64;
    let cached = slot
        .as_ref()
        .map(|loaded| loaded.path == model_path)
        .unwrap_or(false);

    if !cached {
        sllm_llama_sys::backend_init();
        let load_start = Instant::now();
        match Model::load_with(model_path, ModelOptions::default()) {
            Ok(model) => {
                load_ms = load_start.elapsed().as_millis() as u64;
                log::info!("llama smoke: loaded in {load_ms} ms");
                *slot = Some(LoadedModel {
                    path: model_path.to_string(),
                    model,
                });
            }
            Err(error) => return smoke_error("load", &error, 0),
        }
    }

    let loaded = slot.as_ref().expect("model was just loaded");
    let model = &loaded.model;
    log::info!(
        "llama smoke: model={} desc={} n_ctx_train={}",
        cached,
        model.description(),
        model.n_ctx_train()
    );

    let context_start = Instant::now();
    let options = ContextOptions {
        n_ctx: SMOKE_CONTEXT,
        n_batch: 512,
        n_ubatch: 512,
        n_threads: SMOKE_THREADS,
        n_threads_batch: SMOKE_THREADS,
        ..ContextOptions::default()
    };
    let mut ctx = match model.context(&options) {
        Ok(ctx) => ctx,
        Err(error) => return smoke_error("context", &error, load_ms),
    };
    let context_ms = context_start.elapsed().as_millis() as u64;

    let prompt_tokens = match ctx.tokenize(SMOKE_PROMPT, ctx.add_bos(), false) {
        Ok(tokens) => tokens,
        Err(error) => return smoke_error("tokenize", &error, load_ms),
    };
    log::info!(
        "llama smoke: n_ctx={} n_vocab={} prompt_tokens={} ids={:?}",
        ctx.n_ctx(),
        ctx.n_vocab(),
        prompt_tokens.len(),
        prompt_tokens
    );

    let prefill_start = Instant::now();
    if let Err(error) = ctx.decode(&prompt_tokens, 0, 0, false) {
        return smoke_error("prefill", &error, load_ms);
    }
    let prefill_ms = prefill_start.elapsed().as_millis() as u64;

    let mut sampler = match Sampler::new(SamplerConfig {
        top_k: 1,
        top_p: 1.0,
        min_p: 0.0,
        temp: 0.0,
        seed: 42,
    }) {
        Ok(sampler) => sampler,
        Err(error) => return smoke_error("sampler", &error, load_ms),
    };

    let mut tokens = Vec::with_capacity(SMOKE_MAX_TOKENS);
    let mut text = String::new();
    let decode_start = Instant::now();
    for position in (prompt_tokens.len() as i32..).take(SMOKE_MAX_TOKENS) {
        let next = match sampler.sample(&ctx) {
            Ok(token) => token,
            Err(error) => return smoke_error("sample", &error, load_ms),
        };
        if ctx.is_eog(next) {
            log::info!("llama smoke: EOG after {} tokens", tokens.len());
            break;
        }
        tokens.push(next);
        match ctx.token_to_piece(next) {
            Ok(piece) => text.push_str(&piece),
            Err(error) => return smoke_error("detokenize", &error, load_ms),
        }
        if let Err(error) = ctx.decode(&[next], position, 0, false) {
            return smoke_error("decode", &error, load_ms);
        }
    }
    let decode_ms = decode_start.elapsed().as_millis() as u64;
    let total_ms = started.elapsed().as_millis() as u64;

    log::info!(
        "llama smoke ok: load={load_ms}ms ctx={context_ms}ms prefill={prefill_ms}ms \
         decode={decode_ms}ms tokens={} text={text:?}",
        tokens.len()
    );

    json!({
        "ok": true,
        "version": llama_version(),
        "platform": sllm_llama_sys::PLATFORM,
        "model": model_path,
        "modelDesc": model.description(),
        "nCtxTrain": model.n_ctx_train(),
        "loadMs": load_ms,
        "cached": cached,
        "contextMs": context_ms,
        "prefillMs": prefill_ms,
        "decodeMs": decode_ms,
        "prompt": SMOKE_PROMPT,
        "promptTokens": prompt_tokens.len(),
        "tokens": tokens,
        "tokenCount": tokens.len(),
        "text": text,
        "nCtx": ctx.n_ctx(),
        "nVocab": ctx.n_vocab(),
        "threads": SMOKE_THREADS,
        "totalMs": total_ms,
    })
    .to_string()
}

fn llama_unload() {
    let mut slot = model_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot.take().is_some() {
        sllm_llama_sys::backend_free();
        log::info!("llama smoke: model unloaded");
    }
}

fn string_result(env: &mut JNIEnv, value: String) -> jstring {
    match env.new_string(value) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

// ---------------------------------------------------------------------------
// JNI exports
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativeVersion(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    init_log();
    match env.new_string("rust-ok") {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativeModels(
    mut env: JNIEnv,
    _class: JClass,
    dir: JString,
    avail_mb: jni::sys::jint,
) -> jstring {
    init_log();
    let dir: String = env.get_string(&dir).map(|s| s.into()).unwrap_or_default();
    let avail_mb = avail_mb.max(0) as u64;
    log::info!("nativeModels dir={dir} avail={avail_mb}MB");
    let out = scan(&dir, avail_mb);
    string_result(&mut env, out)
}

/// Version string from the linked llama.cpp library (not a Rust stub).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativeLlamaVersion(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    init_log();
    let version = llama_version().to_string();
    log::info!("nativeLlamaVersion: {version}");
    string_result(&mut env, version)
}

/// Load `modelPath`, tokenize a short prompt, run prefill and decode up to four
/// tokens. Blocks the calling Java worker thread; never call it on the UI
/// thread. Returns a JSON report with `ok`, timings, token ids and `error`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativeDeviceSmoke(
    mut env: JNIEnv,
    _class: JClass,
    model_path: JString,
) -> jstring {
    init_log();
    let model_path: String = env
        .get_string(&model_path)
        .map(|s| s.into())
        .unwrap_or_default();
    let out = if model_path.trim().is_empty() {
        json!({
            "ok": false,
            "version": llama_version(),
            "stage": "path",
            "loadMs": 0,
            "prefillMs": 0,
            "decodeMs": 0,
            "tokens": [],
            "tokenCount": 0,
            "text": "",
            "error": "model path is empty",
        })
        .to_string()
    } else {
        llama_smoke(&model_path)
    };
    string_result(&mut env, out)
}

/// Drop the resident model. Called from a worker thread on Activity destroy.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativeLlamaUnload(
    _env: JNIEnv,
    _class: JClass,
) {
    init_log();
    llama_unload();
}

// ---------------------------------------------------------------------------
// Real chat engine
// ---------------------------------------------------------------------------

/// Name of the attached inference backend. `llama` means a real local model.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativeEngine(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    init_log();
    string_result(&mut env, host::engine_name().to_string())
}

/// Start a generation from a JSON request. Returns immediately; events are
/// drained through [`Java_dev_snapdragonllm_MainActivity_nativePoll`].
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
    models_root: JString,
) -> jstring {
    init_log();
    let request: String = env
        .get_string(&request_json)
        .map(|value| value.into())
        .unwrap_or_default();
    let root: String = env
        .get_string(&models_root)
        .map(|value| value.into())
        .unwrap_or_default();
    log::info!("nativeStart root={root}");
    let result = host::start(&request, &root);
    string_result(&mut env, result)
}

/// One JSON event, or `null` when no event is ready.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativePoll(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    match host::poll() {
        Some(event) => string_result(&mut env, event),
        None => std::ptr::null_mut(),
    }
}

/// Load a model without generating; `poll` reports `model` events.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativePrepare(
    mut env: JNIEnv,
    _class: JClass,
    model_id: JString,
    model_path: JString,
    models_root: JString,
) -> jstring {
    init_log();
    let model_id: String = env
        .get_string(&model_id)
        .map(|value| value.into())
        .unwrap_or_default();
    let model_path: String = env
        .get_string(&model_path)
        .map(|value| value.into())
        .unwrap_or_default();
    let root: String = env
        .get_string(&models_root)
        .map(|value| value.into())
        .unwrap_or_default();
    let result = host::prepare(&model_id, &model_path, &root);
    string_result(&mut env, result)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativeCancel(
    _env: JNIEnv,
    _class: JClass,
) {
    host::cancel();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativeStats(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    string_result(&mut env, host::stats())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativeUnload(
    _env: JNIEnv,
    _class: JClass,
) {
    init_log();
    host::unload();
}

/// Android `onTrimMemory` levels translated by `MainActivity`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_snapdragonllm_MainActivity_nativeTrim(
    mut env: JNIEnv,
    _class: JClass,
    level: JString,
) {
    init_log();
    let level: String = env
        .get_string(&level)
        .map(|value| value.into())
        .unwrap_or_default();
    host::trim(&level);
}
