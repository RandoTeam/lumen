//! Native build smoke tests.
//!
//! `cargo test` builds llama.cpp through CMake. On machines without a C++
//! toolchain, use `SLLM_SYS_SKIP_BUILD=1 cargo test` to check the stub API.

#[test]
fn version_is_reported() {
    let version = sllm_llama_sys::version();
    assert!(!version.is_empty());
    println!("llama_version = {version}");
    println!("platform = {}", sllm_llama_sys::PLATFORM);
}

#[test]
fn model_load_without_a_model_fails_cleanly() {
    // No model path is set: the call must fail with a typed error rather than
    // aborting the process.
    let result = sllm_llama_sys::Model::load("definitely-not-a-model.gguf");
    assert!(result.is_err(), "loading a missing model must fail");
    let err = result.err().unwrap();
    println!("expected error: {err} (kind = {:?})", err.kind());
}
