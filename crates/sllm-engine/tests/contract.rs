//! Public contract tests for the mock engine.
//!
//! These exercise the same API the JNI bridge will use: `Arc<dyn Engine>`,
//! streaming, cancellation from another thread, trim and reload.

use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use sllm_engine::*;

fn model(id: &str) -> ModelRef {
    ModelRef::new(id, format!("models/{id}.gguf"))
}

fn cfg() -> EngineConfig {
    EngineConfig::default()
}

fn request(text: &str) -> GenerateRequest {
    GenerateRequest::user_prompt(text)
}

fn loaded(tokens: &[&str]) -> Arc<MockEngine> {
    let engine = Arc::new(
        MockEngine::new()
            .with_tokens(tokens.iter().copied())
            .with_delay(Duration::ZERO),
    );
    engine.load(&model("mock-test"), &cfg()).unwrap();
    engine
}

#[derive(Default)]
struct Collected {
    started: Option<usize>,
    tokens: Vec<(u32, String)>,
    done: Option<GenStats>,
    errors: Vec<EngineError>,
}

impl Collected {
    fn text(&self) -> String {
        self.tokens.iter().map(|(_, text)| text.as_str()).collect()
    }
}

fn record(collected: &mut Collected, event: EngineEvent) {
    match event {
        EngineEvent::Started { prompt_tokens } => collected.started = Some(prompt_tokens),
        EngineEvent::Token { text, id } => collected.tokens.push((id, text)),
        EngineEvent::Done { stats } => collected.done = Some(stats),
        EngineEvent::Error {
            code,
            message,
            retryable,
        } => collected.errors.push(EngineError {
            code,
            message,
            retryable,
        }),
    }
}

fn collect_ok(engine: &MockEngine, request: &GenerateRequest) -> (Collected, GenStats) {
    let mut collected = Collected::default();
    let stats = engine
        .generate(request, &mut |event| record(&mut collected, event))
        .expect("generate should succeed");
    (collected, stats)
}

fn collect_err(engine: &MockEngine, request: &GenerateRequest) -> (Collected, EngineError) {
    let mut collected = Collected::default();
    let error = engine
        .generate(request, &mut |event| record(&mut collected, event))
        .expect_err("generate should fail");
    (collected, error)
}

#[test]
fn new_engine_is_unloaded_and_marked_mock() {
    let engine = MockEngine::new();
    let stats = engine.stats();
    assert_eq!(stats.state, EngineState::Unloaded);
    assert_eq!(stats.model_id, None);
    assert_eq!(stats.trim, TrimLevel::None);
    assert!(!stats.needs_reload);
    assert_eq!(stats.requests, 0);
    assert_eq!(stats.generated_tokens, 0);
    assert!(stats.mock);
}

#[test]
fn load_makes_engine_ready() {
    let engine = loaded(&["x"]);
    let stats = engine.stats();
    assert_eq!(stats.state, EngineState::Ready);
    assert_eq!(stats.model_id.as_deref(), Some("mock-test"));
    assert_eq!(stats.trim, TrimLevel::None);
    assert!(!stats.needs_reload);
}

#[test]
fn streams_all_tokens_in_order() {
    let engine = loaded(&["alpha", " beta", " gamma"]);
    let (collected, stats) = collect_ok(&engine, &request("hello"));

    assert_eq!(collected.started, Some(4));
    assert_eq!(collected.text(), "alpha beta gamma");
    assert_eq!(
        collected
            .tokens
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(collected.errors.is_empty());
    assert_eq!(collected.done, Some(stats.clone()));
    assert_eq!(stats.prompt_tokens, 4);
    assert_eq!(stats.generated_tokens, 3);
    assert!(!stats.canceled);
    assert!(stats.mock);
}

#[test]
fn max_tokens_limits_the_stream() {
    let engine = loaded(&["a", " b", " c", " d"]);
    let mut req = request("x");
    req.max_tokens = 2;

    let (collected, stats) = collect_ok(&engine, &req);
    assert_eq!(collected.text(), "a b");
    assert_eq!(stats.generated_tokens, 2);
    assert!(!stats.canceled);
}

#[test]
fn stop_string_never_reaches_the_sink() {
    let engine = loaded(&["alpha", " beta", " gamma", " delta"]);
    let mut req = request("x");
    req.stop = vec![" gamma".to_string()];

    let (collected, stats) = collect_ok(&engine, &req);
    assert_eq!(collected.text(), "alpha beta");
    assert_eq!(stats.generated_tokens, 3);
    assert!(!stats.canceled);
}

#[test]
fn stop_string_spanning_two_tokens_is_truncated() {
    let engine = loaded(&["alpha", " beta", " gamma", " delta"]);
    let mut req = request("x");
    req.stop = vec![" beta gamma".to_string()];

    let (collected, stats) = collect_ok(&engine, &req);
    assert_eq!(collected.text(), "alpha");
    assert_eq!(stats.generated_tokens, 3);
}

#[test]
fn stop_string_at_the_first_token_emits_no_text() {
    let engine = loaded(&["alpha", " beta"]);
    let mut req = request("x");
    req.stop = vec!["alpha".to_string()];

    let (collected, stats) = collect_ok(&engine, &req);
    assert_eq!(collected.text(), "");
    assert!(collected.tokens.is_empty());
    assert!(collected.done.is_some());
    assert_eq!(stats.generated_tokens, 1);
}

#[test]
fn cancel_from_sink_stops_mid_generation() {
    let engine = loaded(&["a", " b", " c", " d", " e"]);
    let handle = engine.clone();
    let mut pieces = Vec::new();
    let mut done = None;

    let stats = engine
        .generate(&request("x"), &mut |event| match event {
            EngineEvent::Token { text, .. } => {
                pieces.push(text);
                if pieces.len() == 2 {
                    handle.cancel();
                }
            }
            EngineEvent::Done { stats } => done = Some(stats),
            _ => {}
        })
        .unwrap();

    assert_eq!(pieces.concat(), "a b");
    assert!(stats.canceled);
    assert_eq!(stats.generated_tokens, 2);
    assert!(done.unwrap().canceled);
    assert_eq!(engine.stats().state, EngineState::Ready);
}

#[test]
fn cancel_from_another_thread_is_observed() {
    let engine = Arc::new(
        MockEngine::new()
            .with_tokens([" t0", " t1", " t2", " t3"])
            .with_delay(Duration::ZERO),
    );
    engine.load(&model("threaded"), &cfg()).unwrap();

    let worker_engine = engine.clone();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();

    let worker = thread::spawn(move || {
        let mut first = true;
        worker_engine
            .generate(&request("x"), &mut |event| {
                if let EngineEvent::Token { .. } = event
                    && first
                {
                    first = false;
                    started_tx.send(()).unwrap();
                    // Keep the generation in flight until the test has
                    // called cancel from the main thread.
                    release_rx.recv().unwrap();
                }
            })
            .unwrap()
    });

    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("first token from worker");
    engine.cancel();
    release_tx.send(()).unwrap();

    let stats = worker.join().unwrap();
    assert!(stats.canceled);
    assert_eq!(stats.generated_tokens, 1);
    assert_eq!(engine.stats().state, EngineState::Ready);
}

#[test]
fn cancel_is_not_sticky_for_the_next_request() {
    let engine = loaded(&["a", " b", " c"]);
    let handle = engine.clone();
    let mut seen = 0;

    let first = engine
        .generate(&request("x"), &mut |event| {
            if let EngineEvent::Token { .. } = event {
                seen += 1;
                if seen == 1 {
                    handle.cancel();
                }
            }
        })
        .unwrap();
    assert!(first.canceled);

    let (collected, second) = collect_ok(&engine, &request("x"));
    assert_eq!(collected.text(), "a b c");
    assert!(!second.canceled);
}

#[test]
fn moderate_and_running_low_keep_the_engine_ready() {
    let engine = loaded(&["x"]);

    engine.trim(TrimLevel::Moderate);
    let stats = engine.stats();
    assert_eq!(stats.state, EngineState::Ready);
    assert_eq!(stats.trim, TrimLevel::Moderate);
    assert!(!stats.needs_reload);

    engine.trim(TrimLevel::RunningLow);
    let stats = engine.stats();
    assert_eq!(stats.state, EngineState::Ready);
    assert_eq!(stats.trim, TrimLevel::RunningLow);

    let (collected, _) = collect_ok(&engine, &request("x"));
    assert_eq!(collected.text(), "x");
}

#[test]
fn trim_complete_requires_a_reload() {
    let engine = loaded(&["x"]);
    engine.trim(TrimLevel::Complete);

    let stats = engine.stats();
    assert_eq!(stats.state, EngineState::Trimmed);
    assert_eq!(stats.trim, TrimLevel::Complete);
    assert!(stats.needs_reload);
    assert_eq!(stats.model_id.as_deref(), Some("mock-test"));

    let (collected, error) = collect_err(&engine, &request("x"));
    assert_eq!(error.code, codes::E_NO_MODEL);
    assert_eq!(collected.started, None);
    assert_eq!(collected.errors.len(), 1);
    assert_eq!(collected.errors[0].code, codes::E_NO_MODEL);

    engine.load(&model("mock-test"), &cfg()).unwrap();
    assert_eq!(engine.stats().state, EngineState::Ready);
    assert!(!engine.stats().needs_reload);
    let (collected, _) = collect_ok(&engine, &request("x"));
    assert_eq!(collected.text(), "x");
}

#[test]
fn trim_during_generation_is_deferred_until_done() {
    let engine = loaded(&["a", " b"]);
    let handle = engine.clone();
    let mut state_in_done = None;

    let stats = engine
        .generate(&request("x"), &mut |event| match event {
            EngineEvent::Token { .. } => handle.trim(TrimLevel::Complete),
            EngineEvent::Done { .. } => state_in_done = Some(handle.stats().state),
            _ => {}
        })
        .unwrap();

    assert!(!stats.canceled);
    assert_eq!(stats.generated_tokens, 2);
    assert_eq!(state_in_done, Some(EngineState::Trimmed));
    assert_eq!(engine.stats().state, EngineState::Trimmed);
    assert!(engine.stats().needs_reload);
}

#[test]
fn unload_during_generation_is_deferred_until_done() {
    let engine = loaded(&["a", " b"]);
    let handle = engine.clone();
    let mut state_in_done = None;

    engine
        .generate(&request("x"), &mut |event| match event {
            EngineEvent::Token { .. } => handle.unload(),
            EngineEvent::Done { .. } => state_in_done = Some(handle.stats().state),
            _ => {}
        })
        .unwrap();

    assert_eq!(state_in_done, Some(EngineState::Unloaded));
    assert_eq!(engine.stats().state, EngineState::Unloaded);
    assert_eq!(engine.stats().model_id, None);
}

#[test]
fn unload_releases_the_model() {
    let engine = loaded(&["x"]);
    collect_ok(&engine, &request("x"));

    engine.unload();
    let stats = engine.stats();
    assert_eq!(stats.state, EngineState::Unloaded);
    assert_eq!(stats.model_id, None);
    assert!(!stats.needs_reload);
    assert_eq!(stats.trim, TrimLevel::None);

    let (collected, error) = collect_err(&engine, &request("x"));
    assert_eq!(error.code, codes::E_NO_MODEL);
    assert_eq!(collected.errors.len(), 1);
}

#[test]
fn reload_replaces_the_model_and_resets_counters() {
    let engine = loaded(&["a"]);
    collect_ok(&engine, &request("x"));
    assert_eq!(engine.stats().requests, 1);

    engine.load(&model("second"), &cfg()).unwrap();
    let stats = engine.stats();
    assert_eq!(stats.state, EngineState::Ready);
    assert_eq!(stats.model_id.as_deref(), Some("second"));
    assert_eq!(stats.requests, 0);
    assert_eq!(stats.generated_tokens, 0);
    assert!(stats.last.is_none());
}

#[test]
fn stats_accumulate_across_requests() {
    let engine = loaded(&["a", " b"]);
    let (_, first) = collect_ok(&engine, &request("x"));
    let (_, second) = collect_ok(&engine, &request("x"));

    assert_eq!(first.generated_tokens, 2);
    assert_eq!(second.generated_tokens, 2);

    let stats = engine.stats();
    assert_eq!(stats.requests, 2);
    assert_eq!(stats.generated_tokens, 4);
    assert_eq!(stats.last, Some(second));
    assert!(stats.mock);
}

#[test]
fn seed_is_echoed_in_stats() {
    let engine = loaded(&["a"]);
    let mut req = request("x");
    req.seed = 42;

    let (_, stats) = collect_ok(&engine, &req);
    assert_eq!(stats.seed, 42);
}

#[test]
fn nested_generate_is_rejected_as_busy() {
    let engine = loaded(&["a", " b"]);
    let handle = engine.clone();
    let mut nested = None;

    let stats = engine
        .generate(&request("x"), &mut |event| {
            if let EngineEvent::Token { .. } = event
                && nested.is_none()
            {
                let inner = request("y");
                let mut noop = |_event: EngineEvent| {};
                nested = Some(handle.generate(&inner, &mut noop).unwrap_err());
            }
        })
        .unwrap();

    assert_eq!(nested.unwrap().code, codes::E_BUSY);
    assert_eq!(stats.generated_tokens, 2);
}

#[test]
fn invalid_request_emits_error_and_keeps_ready() {
    let engine = loaded(&["a"]);
    let mut req = request("x");
    req.sampler.temperature = -1.0;

    let (collected, error) = collect_err(&engine, &req);
    assert_eq!(error.code, codes::E_CONFIG);
    assert_eq!(collected.started, None);
    assert_eq!(collected.errors.len(), 1);
    assert_eq!(collected.errors[0].code, codes::E_CONFIG);

    let stats = engine.stats();
    assert_eq!(stats.state, EngineState::Ready);
    assert_eq!(stats.requests, 0);
}

#[test]
fn invalid_config_does_not_replace_the_loaded_model() {
    let engine = loaded(&["a"]);
    let mut bad = cfg();
    bad.threads = 0;

    let error = engine.load(&model("other"), &bad).unwrap_err();
    assert_eq!(error.code, codes::E_CONFIG);
    assert_eq!(engine.stats().model_id.as_deref(), Some("mock-test"));
    assert_eq!(engine.stats().state, EngineState::Ready);
}

#[test]
fn empty_script_finishes_cleanly() {
    let engine = loaded(&[]);
    let (collected, stats) = collect_ok(&engine, &request("x"));

    assert_eq!(collected.started, Some(4));
    assert!(collected.tokens.is_empty());
    assert!(collected.done.is_some());
    assert_eq!(stats.generated_tokens, 0);
    assert!(!stats.canceled);
}

#[test]
fn core_error_codes_pass_through() {
    let error = EngineError::from(SllmError::new(ErrorCode::QuantUnsupported, "nope"));
    assert_eq!(error.code, "E_QUANT_UNSUPPORTED");
    assert!(!error.retryable);
}

#[test]
fn mock_engine_is_send_sync_and_usable_as_a_trait_object() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MockEngine>();

    let engine: Arc<dyn Engine> = Arc::new(MockEngine::new());
    engine.load(&model("dyn"), &cfg()).unwrap();
    assert_eq!(engine.stats().state, EngineState::Ready);
}
