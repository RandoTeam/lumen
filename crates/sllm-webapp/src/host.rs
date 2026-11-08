//! Real inference host shared by the JNI bridge.
//!
//! The WebView calls `start` with a JSON request, then drains `poll` from a
//! timer. Generation runs on a Rust worker thread, so the JavaScript thread is
//! never blocked by model loading or decoding.
//!
//! There is no mock backend here. If a model cannot be loaded, the UI receives
//! an `error` event with the real llama.cpp message.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::Instant;

use serde::Deserialize;
use serde_json::{Value, json};
use sllm_engine::{
    Engine, EngineConfig, EngineEvent, GenStats, GenerateRequest, LlamaEngine, LoraRef, Message,
    ModelRef, Role, SamplerConfig,
};
use sllm_model::{FileRole, ModelRegistry};

/// Drop the oldest events instead of growing without bound if the WebView stops
/// polling (for example while the Activity is in the background).
const MAX_QUEUE: usize = 4096;

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessagePayload {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartPayload {
    #[serde(default)]
    messages: Vec<MessagePayload>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    top_k: Option<u32>,
    #[serde(default)]
    min_p: Option<f32>,
    #[serde(default)]
    repeat_penalty: Option<f32>,
    #[serde(default)]
    seed: Option<i64>,
    #[serde(default)]
    stop: Vec<String>,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    model_path: Option<String>,
}

impl StartPayload {
    fn into_request(self) -> GenerateRequest {
        let sampler = SamplerConfig {
            temperature: self.temperature.unwrap_or(0.7),
            top_p: self.top_p.unwrap_or(0.95),
            top_k: self.top_k.unwrap_or(40),
            min_p: self.min_p.unwrap_or(0.05),
            repeat_penalty: self.repeat_penalty.unwrap_or(1.1),
        };
        let messages = self
            .messages
            .into_iter()
            .map(|message| Message::new(role(&message.role), message.content))
            .collect();
        GenerateRequest {
            messages,
            sampler,
            max_tokens: self.max_tokens.unwrap_or(512),
            stop: self.stop,
            seed: self.seed.unwrap_or(-1),
        }
    }
}

fn role(value: &str) -> Role {
    match value {
        "assistant" => Role::Assistant,
        "system" => Role::System,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

#[derive(Debug, Clone)]
struct Loaded {
    id: String,
    path: PathBuf,
}

struct Shared {
    queue: Mutex<VecDeque<String>>,
    loaded: Mutex<Option<Loaded>>,
    busy: AtomicBool,
    cancel: AtomicBool,
    generation: AtomicU64,
}

impl Shared {
    fn push(&self, generation: u64, mut event: Value) {
        if let Some(object) = event.as_object_mut() {
            object.insert("generationId".to_string(), json!(generation));
        }
        let text = event.to_string();
        let mut queue = lock(&self.queue);
        if queue.len() >= MAX_QUEUE {
            queue.pop_front();
        }
        queue.push_back(text);
    }

    fn push_event(&self, generation: u64, event: EngineEvent) {
        match serde_json::to_value(event) {
            Ok(value) => self.push(generation, value),
            Err(error) => self.push(
                generation,
                json!({
                    "event": "error",
                    "code": "E_EVENT",
                    "message": format!("could not serialize engine event: {error}"),
                    "retryable": false,
                }),
            ),
        }
    }

    fn push_error(&self, generation: u64, code: &str, message: impl Into<String>) {
        self.push(
            generation,
            json!({
                "event": "error",
                "code": code,
                "message": message.into(),
                "retryable": code == "E_BUSY",
            }),
        );
    }

    fn push_model(&self, generation: u64, state: &str, model_id: &str, message: impl Into<String>) {
        self.push(
            generation,
            json!({
                "event": "model",
                "state": state,
                "modelId": model_id,
                "message": message.into(),
            }),
        );
    }

    fn push_canceled(&self, generation: u64, prompt_tokens: usize, seed: i64, started: Instant) {
        let elapsed = started.elapsed().as_millis() as u64;
        let stats = GenStats {
            prompt_tokens,
            generated_tokens: 0,
            ttft_ms: 0,
            prefill_ms: 0,
            decode_ms: 0,
            tok_per_s: 0.0,
            seed,
            canceled: true,
            mock: false,
        };
        let _ = elapsed;
        self.push_event(generation, EngineEvent::Done { stats });
    }
}

struct Host {
    engine: Arc<dyn Engine>,
    shared: Arc<Shared>,
    config: EngineConfig,
    next_generation: u64,
    worker: Option<JoinHandle<()>>,
}

impl Host {
    fn new() -> Self {
        let threads = std::thread::available_parallelism()
            .map(|count| count.get().min(4) as u32)
            .unwrap_or(4)
            .max(1);
        let config = EngineConfig {
            threads,
            ctx: 4096,
            ..EngineConfig::default()
        };
        Self {
            engine: Arc::new(LlamaEngine::new()),
            shared: Arc::new(Shared {
                queue: Mutex::new(VecDeque::new()),
                loaded: Mutex::new(None),
                busy: AtomicBool::new(false),
                cancel: AtomicBool::new(false),
                generation: AtomicU64::new(0),
            }),
            config,
            next_generation: 1,
            worker: None,
        }
    }

    fn reap_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                self.worker = Some(worker);
            }
        }
    }
}

static HOST: OnceLock<Mutex<Host>> = OnceLock::new();

fn host() -> &'static Mutex<Host> {
    HOST.get_or_init(|| Mutex::new(Host::new()))
}

pub fn engine_name() -> &'static str {
    "llama"
}

fn ack_ok(generation: u64, model_id: &str) -> String {
    json!({
        "ok": true,
        "generationId": generation,
        "modelId": model_id,
        "mock": false,
    })
    .to_string()
}

fn ack_error(code: &str, message: impl Into<String>) -> String {
    json!({
        "ok": false,
        "error": {
            "code": code,
            "message": message.into(),
            "retryable": code == "E_BUSY" || code == "E_LOADING",
        },
    })
    .to_string()
}

fn model_from_pack(root: &str, pack: &sllm_model::Pack) -> Result<ModelRef, String> {
    if !pack.supported {
        return Err(pack.reason(0));
    }
    let path = Path::new(root).join(&pack.base_path);
    if !path.is_file() {
        return Err(format!(
            "model '{}' is incomplete: {} is missing",
            pack.id,
            path.display()
        ));
    }
    let mut model = ModelRef::new(pack.id.clone(), path).with_name(pack.name.clone());
    for file in &pack.files {
        if file.role == FileRole::Lora {
            let adapter = Path::new(root).join(&file.path);
            if adapter.is_file() {
                model = model.with_lora(LoraRef::new(adapter));
            }
        }
    }
    Ok(model)
}

fn resolve_model(root: &str, id: Option<&str>, explicit: Option<&str>) -> Result<ModelRef, String> {
    let requested = id.filter(|id| !id.trim().is_empty());
    let root_is_dir = !root.trim().is_empty() && Path::new(root).is_dir();

    // Prefer the registry when both an id and a models root are available: this
    // is what attaches companion LoRA adapters. The absolute path from the web
    // UI is only a fallback for models outside the registry.
    if root_is_dir
        && let Some(id) = requested
        && let Ok(registry) = ModelRegistry::discover(root)
        && let Some(pack) = registry.get(id)
    {
        return model_from_pack(root, pack);
    }

    if let Some(path) = explicit.filter(|path| !path.trim().is_empty()) {
        let path = PathBuf::from(path);
        if !path.is_file() {
            return Err(format!("model file does not exist: {}", path.display()));
        }
        let id = requested
            .map(str::to_string)
            .or_else(|| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "model".to_string());
        return Ok(ModelRef::new(id, path));
    }

    if !root_is_dir {
        if root.trim().is_empty() {
            return Err("models directory is empty (Activity did not pass a path)".to_string());
        }
        return Err(format!("models directory does not exist: {root}"));
    }

    let registry =
        ModelRegistry::discover(root).map_err(|error| format!("cannot scan {root}: {error}"))?;
    let pack = match requested {
        Some(id) => registry.get(id).or_else(|| {
            let needle = id.to_ascii_lowercase();
            registry.packs().iter().find(|pack| {
                pack.id.to_ascii_lowercase().contains(&needle)
                    || pack.name.to_ascii_lowercase().contains(&needle)
            })
        }),
        None => registry.packs().first(),
    }
    .ok_or_else(|| {
        let available: Vec<&str> = registry
            .packs()
            .iter()
            .map(|pack| pack.id.as_str())
            .collect();
        format!(
            "model '{}' is not installed; available: {}",
            requested.unwrap_or("(none)"),
            if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            }
        )
    })?;
    model_from_pack(root, pack)
}

/// Start a background generation. Returns immediately with the generation id.
pub fn start(request_json: &str, models_root: &str) -> String {
    let payload: StartPayload = match serde_json::from_str(request_json) {
        Ok(payload) => payload,
        Err(error) => return ack_error("E_CONFIG", format!("invalid start request: {error}")),
    };
    let model_id = payload.model_id.clone();
    let model_path = payload.model_path.clone();
    let request = payload.into_request();
    if request.messages.is_empty() {
        return ack_error("E_CONFIG", "messages are empty");
    }
    if let Err(error) = request.validate() {
        return ack_error(&error.code, error.message.clone());
    }

    let model = match resolve_model(models_root, model_id.as_deref(), model_path.as_deref()) {
        Ok(model) => model,
        Err(message) => return ack_error("E_MODEL", message),
    };

    let mut host = lock(host());
    host.reap_worker();
    if host.shared.busy.load(Ordering::Acquire) {
        return ack_error("E_BUSY", "a generation is already running");
    }

    let loaded_matches = {
        let loaded = lock(&host.shared.loaded);
        loaded
            .as_ref()
            .map(|loaded| loaded.id == model.id && loaded.path == model.path)
            .unwrap_or(false)
    };

    let generation = host.next_generation;
    host.next_generation = host.next_generation.wrapping_add(1).max(1);
    host.shared.generation.store(generation, Ordering::Release);
    host.shared.cancel.store(false, Ordering::Release);
    host.shared.busy.store(true, Ordering::Release);
    {
        let mut queue = lock(&host.shared.queue);
        queue.clear();
    }

    let shared = Arc::clone(&host.shared);
    let config = host.config.clone();
    let model_clone = model.clone();
    let request_clone = request.clone();
    let engine = Arc::clone(&host.engine);
    let name = model.display_name();
    let id = model.id.clone();

    let worker = std::thread::Builder::new()
        .name("sllm-generate".to_string())
        .spawn(move || {
            let _busy = BusyGuard {
                shared: Arc::clone(&shared),
            };
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                run_generation(
                    &shared,
                    engine,
                    &config,
                    &model_clone,
                    &request_clone,
                    generation,
                    loaded_matches,
                    &name,
                );
            }));
            if outcome.is_err() {
                shared.push_error(generation, "E_PANIC", "the inference worker panicked");
                log::error!("generation {generation} panicked");
            }
        });

    match worker {
        Ok(worker) => {
            host.worker = Some(worker);
            ack_ok(generation, &id)
        }
        Err(error) => {
            host.shared.busy.store(false, Ordering::Release);
            ack_error(
                "E_INTERNAL",
                format!("cannot start inference worker: {error}"),
            )
        }
    }
}

/// Load a model into the engine without generating. Used by the Models screen.
pub fn prepare(model_id: &str, model_path: &str, models_root: &str) -> String {
    let model = match resolve_model(
        models_root,
        Some(model_id).filter(|id| !id.trim().is_empty()),
        Some(model_path).filter(|path| !path.trim().is_empty()),
    ) {
        Ok(model) => model,
        Err(message) => return ack_error("E_MODEL", message),
    };

    let mut host = lock(host());
    host.reap_worker();
    if host.shared.busy.load(Ordering::Acquire) {
        return ack_error("E_BUSY", "a generation is already running");
    }

    let loaded_matches = {
        let loaded = lock(&host.shared.loaded);
        loaded
            .as_ref()
            .map(|loaded| loaded.id == model.id && loaded.path == model.path)
            .unwrap_or(false)
    };
    if loaded_matches {
        return json!({
            "ok": true,
            "modelId": model.id,
            "state": "ready",
            "mock": false,
        })
        .to_string();
    }

    host.shared.busy.store(true, Ordering::Release);
    host.shared.cancel.store(false, Ordering::Release);

    let shared = Arc::clone(&host.shared);
    let engine = Arc::clone(&host.engine);
    let config = host.config.clone();
    let id = model.id.clone();
    let name = model.display_name();

    let worker = std::thread::Builder::new()
        .name("sllm-load".to_string())
        .spawn(move || {
            let _busy = BusyGuard {
                shared: Arc::clone(&shared),
            };
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let previous = lock(&shared.loaded).take();
                if previous.is_some() {
                    engine.unload();
                }
                shared.push_model(0, "loading", &model.id, format!("loading {name}"));
                let started = Instant::now();
                match engine.load(&model, &config) {
                    Ok(()) => {
                        let load_ms = started.elapsed().as_millis() as u64;
                        *lock(&shared.loaded) = Some(Loaded {
                            id: model.id.clone(),
                            path: model.path.clone(),
                        });
                        shared.push_model(
                            0,
                            "ready",
                            &model.id,
                            format!("{name} loaded in {load_ms} ms"),
                        );
                    }
                    Err(error) => {
                        log::error!("prepare: load failed: {error}");
                        shared.push_model(0, "error", &model.id, error.message.clone());
                    }
                }
            }));
            if outcome.is_err() {
                shared.push_model(0, "error", &model.id, "the loader panicked");
            }
        });

    match worker {
        Ok(worker) => {
            host.worker = Some(worker);
            json!({
                "ok": true,
                "modelId": id,
                "state": "loading",
                "mock": false,
            })
            .to_string()
        }
        Err(error) => {
            host.shared.busy.store(false, Ordering::Release);
            ack_error("E_INTERNAL", format!("cannot start loader: {error}"))
        }
    }
}

struct BusyGuard {
    shared: Arc<Shared>,
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.shared.busy.store(false, Ordering::Release);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_generation(
    shared: &Shared,
    engine: Arc<dyn Engine>,
    config: &EngineConfig,
    model: &ModelRef,
    request: &GenerateRequest,
    generation: u64,
    loaded_matches: bool,
    name: &str,
) {
    let started = Instant::now();
    if !loaded_matches {
        // Release the previous model first: on a 6 GB phone loading the next
        // model while the old session is still resident can double peak RSS.
        let previous = lock(&shared.loaded).take();
        if previous.is_some() {
            engine.unload();
        }
        shared.push_model(generation, "loading", &model.id, format!("loading {name}"));
        let load_start = Instant::now();
        match engine.load(model, config) {
            Ok(()) => {
                let load_ms = load_start.elapsed().as_millis() as u64;
                *lock(&shared.loaded) = Some(Loaded {
                    id: model.id.clone(),
                    path: model.path.clone(),
                });
                log::info!(
                    "generation {generation}: loaded {} in {load_ms} ms",
                    model.path.display()
                );
                shared.push_model(
                    generation,
                    "ready",
                    &model.id,
                    format!("{name} loaded in {load_ms} ms"),
                );
            }
            Err(error) => {
                log::error!("generation {generation}: load failed: {error}");
                shared.push_error(generation, &error.code, error.message.clone());
                return;
            }
        }
    }

    if shared.cancel.load(Ordering::Acquire) {
        shared.push_canceled(generation, 0, request.seed, started);
        return;
    }

    let mut sink = |event: EngineEvent| shared.push_event(generation, event);
    match engine.generate(request, &mut sink) {
        Ok(stats) => {
            log::info!(
                "generation {generation}: {} tokens, {:.1} tok/s, ttft {} ms",
                stats.generated_tokens,
                stats.tok_per_s,
                stats.ttft_ms
            );
        }
        // LlamaEngine emits its own error event before returning Err for every
        // failure except E_BUSY, which cannot happen on this single worker.
        Err(error) => log::error!("generation {generation}: {error}"),
    }
}

/// Pop one JSON event, or `None` when the queue is empty.
pub fn poll() -> Option<String> {
    let host = lock(host());
    lock(&host.shared.queue).pop_front()
}

pub fn cancel() {
    let host = lock(host());
    host.shared.cancel.store(true, Ordering::Release);
    host.engine.cancel();
}

pub fn stats() -> String {
    let host = lock(host());
    let stats = host.engine.stats();
    let mut value = serde_json::to_value(stats).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "busy".to_string(),
            json!(host.shared.busy.load(Ordering::Acquire)),
        );
        object.insert(
            "generationId".to_string(),
            json!(host.shared.generation.load(Ordering::Acquire)),
        );
    }
    value.to_string()
}

pub fn unload() {
    let host = lock(host());
    host.shared.cancel.store(true, Ordering::Release);
    host.engine.unload();
    *lock(&host.shared.loaded) = None;
    lock(&host.shared.queue).clear();
    log::info!("engine unloaded");
}

/// Apply an Android `onTrimMemory` level to the live engine.
pub fn trim(level: &str) {
    use sllm_engine::TrimLevel;

    let level = match level {
        "moderate" => TrimLevel::Moderate,
        "running_low" => TrimLevel::RunningLow,
        "complete" => TrimLevel::Complete,
        _ => return,
    };
    let host = lock(host());
    host.engine.trim(level);
    if level >= TrimLevel::Complete {
        *lock(&host.shared.loaded) = None;
    }
    log::info!("engine trim: {}", level.as_str());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_mapping_is_total() {
        assert_eq!(role("assistant"), Role::Assistant);
        assert_eq!(role("system"), Role::System);
        assert_eq!(role("tool"), Role::Tool);
        assert_eq!(role("anything-else"), Role::User);
    }

    #[test]
    fn request_defaults_are_real_sampler_values() {
        let payload: StartPayload =
            serde_json::from_str(r#"{"messages":[{"role":"user","content":"hi"}]}"#).unwrap();
        let request = payload.into_request();
        assert_eq!(request.max_tokens, 512);
        assert_eq!(request.messages.len(), 1);
        assert!((request.sampler.temperature - 0.7).abs() < 1e-6);
    }

    #[test]
    fn missing_explicit_path_is_reported() {
        let error = resolve_model("", None, Some("Z:/does/not/exist.gguf")).unwrap_err();
        assert!(error.contains("does not exist"));
    }

    #[test]
    fn ternary_pack_attaches_its_lora_adapter() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/ternary");
        if !root.join("Ternary-Bonsai-2-4B-Q2_0.gguf").is_file() {
            return;
        }
        let model = resolve_model(
            root.to_str().unwrap(),
            Some("ternary-bonsai-2-4b-q2-0"),
            None,
        )
        .expect("resolve ternary pack");
        assert_eq!(model.loras.len(), 1, "the pack ships one LoRA adapter");

        // The web UI always sends an absolute modelPath too. The registry must
        // still win so the adapter is not silently dropped.
        let absolute = root.join("Ternary-Bonsai-2-4B-Q2_0.gguf");
        let model = resolve_model(
            root.to_str().unwrap(),
            Some("ternary-bonsai-2-4b-q2-0"),
            Some(absolute.to_str().unwrap()),
        )
        .expect("resolve ternary pack with explicit path");
        assert_eq!(model.loras.len(), 1);
    }

    /// Opt-in load check for the 11.69 GB Clover pack. Generation is not
    /// attempted here; the point is the clover_moe graph and mmap load path.
    #[test]
    fn clover_pack_loads_when_opted_in() {
        use std::time::{Duration, Instant};

        if std::env::var_os("SLLM_TEST_CLOVER").is_none() {
            return;
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../models")
            .to_string_lossy()
            .into_owned();
        let ack: Value = serde_json::from_str(&prepare(
            "clover-35b-a3b-preview-apex-i-nanoplus-v2-1",
            "",
            &root,
        ))
        .unwrap();
        assert_eq!(ack["ok"], true, "prepare failed: {ack}");

        let deadline = Instant::now() + Duration::from_secs(600);
        let mut ready = false;
        let mut failure = None;
        while Instant::now() < deadline && !ready && failure.is_none() {
            match poll() {
                Some(raw) => {
                    let event: Value = serde_json::from_str(&raw).unwrap();
                    match event["state"].as_str() {
                        Some("ready") => ready = true,
                        Some("error") => failure = Some(event["message"].to_string()),
                        _ => {}
                    }
                }
                None => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        unload();
        assert!(failure.is_none(), "clover load failed: {failure:?}");
        assert!(ready, "clover load timed out");
    }

    /// Opt-in end-to-end check through the same code path the JNI bridge uses.
    /// Set `SLLM_TEST_MODEL` to a small GGUF; otherwise the test is a no-op.
    #[test]
    fn real_generation_streams_through_the_host_queue() {
        use std::time::{Duration, Instant};

        let mut request = json!({
            "messages": [{ "role": "user", "content": "Say hello in one short sentence." }],
            "maxTokens": 8,
            "temperature": 0.0,
            "seed": 42,
        });
        let mut root = String::new();
        if let Some(path) = std::env::var_os("SLLM_TEST_MODEL") {
            request["modelPath"] = json!(PathBuf::from(path).to_string_lossy());
        } else if std::env::var_os("SLLM_TEST_TERNARY").is_some() {
            root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../models/ternary")
                .to_string_lossy()
                .into_owned();
            request["modelId"] = json!("ternary-bonsai-2-4b-q2-0");
        } else {
            return;
        }

        let ack: Value = serde_json::from_str(&start(&request.to_string(), &root)).unwrap();
        assert_eq!(ack["ok"], true, "start failed: {ack}");

        let deadline = Instant::now() + Duration::from_secs(120);
        let mut text = String::new();
        let mut done = false;
        while Instant::now() < deadline && !done {
            match poll() {
                Some(raw) => {
                    let event: Value = serde_json::from_str(&raw).unwrap();
                    match event["event"].as_str() {
                        Some("token") => text.push_str(event["text"].as_str().unwrap_or("")),
                        Some("done") => done = true,
                        Some("error") => panic!("generation failed: {event}"),
                        _ => {}
                    }
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        unload();
        assert!(done, "generation timed out");
        assert!(!text.is_empty(), "model produced no text");
    }
}
