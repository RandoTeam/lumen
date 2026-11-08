//! Prints the linked llama.cpp version and, when `SLLM_TEST_MODEL` is set,
//! performs a real load/tokenize/decode/sample smoke test.
//!
//! ```powershell
//! $env:SLLM_TEST_MODEL = "..\..\models\tiny\Qwen2.5-0.5B-Instruct-Q4_K_M.gguf"
//! cargo run --manifest-path crates/sllm-llama-sys/Cargo.toml --example version
//! ```

use std::env;
use std::error::Error;
use std::time::Instant;

use sllm_llama_sys::{
    ContextOptions, Model, ModelOptions, Sampler, SamplerConfig, backend_free, backend_init,
};

fn main() {
    if let Err(err) = run() {
        eprintln!("sllm-llama-sys smoke test failed: {err}");
        if let Some(source) = err.source() {
            eprintln!("  caused by: {source}");
        }
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    println!("llama_version = {}", sllm_llama_sys::version());
    println!(
        "platform = {} (android = {})",
        sllm_llama_sys::PLATFORM,
        sllm_llama_sys::is_android()
    );

    backend_init();

    let model_path = match env::var("SLLM_TEST_MODEL") {
        Ok(path) if !path.is_empty() => path,
        _ => {
            println!("SLLM_TEST_MODEL is not set; skipping model smoke test");
            backend_free();
            return Ok(());
        }
    };

    // Model and context must be dropped before backend_free().
    let result = smoke(&model_path);
    backend_free();
    result
}

fn smoke(model_path: &str) -> Result<(), Box<dyn Error>> {
    println!("loading model = {model_path}");
    let started = Instant::now();
    let model = Model::load_with(model_path, ModelOptions::default())?;
    println!("loaded in {:.2?}", started.elapsed());
    println!("model description = {}", model.description());

    let ctx_options = ContextOptions {
        n_ctx: 512,
        n_batch: 128,
        n_ubatch: 128,
        ..Default::default()
    };
    let mut ctx = model.context(&ctx_options)?;
    println!("n_ctx = {}, n_vocab = {}", ctx.n_ctx(), ctx.n_vocab());

    let prompt = env::var("SLLM_TEST_PROMPT").unwrap_or_else(|_| "Hello".to_string());
    let tokens = ctx.tokenize(&prompt, true, false)?;
    println!("prompt = {prompt:?}");
    println!("tokens ({}): {tokens:?}", tokens.len());

    let started = Instant::now();
    ctx.decode(&tokens, 0, 0, false)?;
    println!("decode took {:.2?}", started.elapsed());
    println!("logits = {}", ctx.logits().len());

    let mut sampler = Sampler::new(SamplerConfig {
        temp: 0.0,
        ..Default::default()
    })?;
    let next = sampler.sample(&ctx)?;
    let piece = ctx.token_to_piece(next)?;
    println!("greedy next token = {next} ({piece:?})");

    let text = ctx.detokenize(&tokens)?;
    println!("detokenized prompt = {text:?}");
    Ok(())
}
