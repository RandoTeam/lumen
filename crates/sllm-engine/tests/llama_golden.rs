//! Real-model tests for `LlamaEngine`.
//!
//! They are compiled only with `--features llama`. Every test skips cleanly
//! when `SLLM_TEST_MODEL` is unset (or points at a missing file), so CI without
//! the 397 MB Qwen file stays green.
//!
//! Set `SLLM_GOLDEN_TEXT` to compare the greedy completion byte-for-byte with
//! text produced by `llama-cli` on the same llama.cpp tree. Without it the test
//! still checks that two runs are identical.

#![cfg(feature = "llama")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sllm_engine::{
    Engine, EngineConfig, EngineEvent, EngineState, GenerateRequest, LlamaEngine, Message,
    ModelRef, SamplerConfig, TrimLevel,
};

const PROMPT: &str = "Say hello in one short sentence.";

fn model_path() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("SLLM_TEST_MODEL")?);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!(
            "skip: SLLM_TEST_MODEL does not point at a file: {}",
            path.display()
        );
        None
    }
}

fn engine_config() -> EngineConfig {
    EngineConfig {
        threads: 4,
        ctx: 512,
        batch: 128,
        ..EngineConfig::default()
    }
}

fn greedy_request(max_tokens: u32) -> GenerateRequest {
    GenerateRequest {
        messages: vec![Message::user(PROMPT)],
        sampler: SamplerConfig {
            temperature: 0.0,
            top_p: 1.0,
            top_k: 0,
            min_p: 0.0,
            repeat_penalty: 1.0,
        },
        max_tokens,
        stop: Vec::new(),
        seed: 42,
    }
}

fn generate_text(
    engine: &LlamaEngine,
    request: &GenerateRequest,
) -> (String, sllm_engine::GenStats) {
    let mut text = String::new();
    let mut done = 0usize;
    let stats = engine
        .generate(request, &mut |event| match event {
            EngineEvent::Token { text: piece, .. } => text.push_str(&piece),
            EngineEvent::Done { .. } => done += 1,
            _ => {}
        })
        .expect("generation failed");
    assert_eq!(done, 1, "exactly one Done event");
    (text, stats)
}

#[test]
fn qwen_greedy_is_deterministic() {
    let Some(path) = model_path() else {
        return;
    };

    let engine = LlamaEngine::new();
    engine
        .load(
            &ModelRef::new("qwen2.5-0.5b", path.as_path()),
            &engine_config(),
        )
        .expect("load");

    let request = greedy_request(8);
    let (first, stats) = generate_text(&engine, &request);
    let (second, _) = generate_text(&engine, &request);

    eprintln!("golden text: {first:?}");
    eprintln!(
        "prompt={} generated={} ttft={}ms prefill={}ms decode={}ms {:.1} tok/s seed={} mock={}",
        stats.prompt_tokens,
        stats.generated_tokens,
        stats.ttft_ms,
        stats.prefill_ms,
        stats.decode_ms,
        stats.tok_per_s,
        stats.seed,
        stats.mock
    );

    assert!(!first.is_empty(), "model produced no text");
    assert!(!stats.mock, "LlamaEngine must report mock = false");
    assert_eq!(stats.seed, 42);
    assert_eq!(first, second, "greedy decoding must be reproducible");

    if let Ok(expected) = std::env::var("SLLM_GOLDEN_TEXT") {
        assert_eq!(
            first, expected,
            "completion differs from llama-cli golden text"
        );
    }

    let final_stats = engine.stats();
    assert_eq!(final_stats.state, EngineState::Ready);
    assert_eq!(final_stats.requests, 2);
    assert_eq!(
        final_stats.generated_tokens as usize,
        stats.generated_tokens * 2
    );
}

#[test]
fn cancel_trim_and_reload() {
    let Some(path) = model_path() else {
        return;
    };

    let engine = Arc::new(LlamaEngine::new());
    engine
        .load(
            &ModelRef::new("qwen2.5-0.5b", path.as_path()),
            &engine_config(),
        )
        .expect("load");

    // Long enough that cancel arrives mid-stream.
    let request = greedy_request(256);
    let worker = {
        let engine = Arc::clone(&engine);
        let request = request.clone();
        std::thread::spawn(move || {
            let mut seen_done = 0usize;
            let stats = engine
                .generate(&request, &mut |event| {
                    if matches!(event, EngineEvent::Done { .. }) {
                        seen_done += 1;
                    }
                })
                .expect("generation");
            (stats, seen_done)
        })
    };
    std::thread::sleep(Duration::from_millis(150));
    engine.cancel();
    let (stats, seen_done) = worker.join().expect("join");
    assert_eq!(seen_done, 1);
    assert!(stats.canceled || stats.generated_tokens > 0);

    // RunningLow drops KV but keeps the weights; the next generate recreates
    // the context.
    engine.trim(TrimLevel::RunningLow);
    assert_eq!(engine.stats().state, EngineState::Ready);
    let (text, _) = generate_text(&engine, &greedy_request(8));
    assert!(!text.is_empty());

    // Complete releases the weights, keeps the model id, and needs a reload.
    engine.trim(TrimLevel::Complete);
    let trimmed = engine.stats();
    assert_eq!(trimmed.state, EngineState::Trimmed);
    assert!(trimmed.needs_reload);
    assert_eq!(trimmed.model_id.as_deref(), Some("qwen2.5-0.5b"));
    assert_eq!(
        engine
            .generate(&greedy_request(8), &mut |_| {})
            .unwrap_err()
            .code,
        "E_NO_MODEL"
    );

    engine
        .load(
            &ModelRef::new("qwen2.5-0.5b", path.as_path()),
            &engine_config(),
        )
        .expect("reload");
    assert_eq!(engine.stats().state, EngineState::Ready);

    engine.unload();
    let unloaded = engine.stats();
    assert_eq!(unloaded.state, EngineState::Unloaded);
    assert!(!unloaded.needs_reload);
    assert!(unloaded.model_id.is_none());
}
