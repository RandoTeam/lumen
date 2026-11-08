# sllm-llama-sys

Minimal FFI crate for the pinned llama.cpp fork
(`external/llama-clover-src`, `4838ffb`, PR #31306).

It builds `llama`, `ggml`, `llama-common` and `mtmd` with CMake plus a small
C++ exception-safe shim, runs bindgen over `llama.h` / `ggml.h` /
`ggml-backend.h`, and exposes a thin safe API:

```rust
use sllm_llama_sys::{ContextOptions, Model, ModelOptions, Sampler, SamplerConfig};

let model = Model::load_with("model.gguf", ModelOptions::default())?;
let mut ctx = model.context(&ContextOptions { n_ctx: 2048, ..Default::default() })?;
let tokens = ctx.tokenize("Hello", true, false)?;
ctx.decode(&tokens, 0, 0, false)?;

let mut sampler = Sampler::new(SamplerConfig::default())?;
let next = sampler.sample(&ctx)?; // sample() already accepts the token
let piece = ctx.token_to_piece(next)?;
# Ok::<(), sllm_llama_sys::Error>(())
```

## Build

Static build by default. The crate is standalone (`[workspace]`) and is not a
member of the root SnapdragonLLM workspace.

| Variable | Meaning |
|---|---|
| `SLLM_LLAMA_SRC` | source root (default `../../external/llama-clover-src`) |
| `SLLM_SYS_SKIP_BUILD=1` | no CMake/bindgen; API returns `ErrorKind::NotBuilt` |
| `ANDROID_NDK_HOME` | NDK 28.x root for `--target aarch64-linux-android` |
| `SLLM_CPU_VARIANT` | `baseline` (default), `v82`, `v86`, `v90` |
| `SLLM_OPENCL=1` | build the OpenCL backend (also `--features opencl`) |
| `SLLM_SHARED=1` | shared llama.cpp libs instead of static |
| `SLLM_CMAKE_GENERATOR` | e.g. `Ninja` |
| `SLLM_CMAKE_PROFILE` | native build type, default `Release` |
| `LIBCLANG_PATH` | directory with `libclang.dll`/`.so`/`.dylib` |
| `SLLM_TEST_MODEL` | GGUF path for `examples/version.rs` |

Windows host (MSVC shell required for linking):

```powershell
cmd /c "call `"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat`" >nul && cargo test --manifest-path crates\sllm-llama-sys\Cargo.toml"
```

Android:

```powershell
$env:ANDROID_NDK_HOME = "$env:LOCALAPPDATA\Android\Sdk\ndk\28.2.13676358"
cargo build --manifest-path crates/sllm-llama-sys/Cargo.toml --target aarch64-linux-android
```

`build.rs` auto-detects the Python `libclang` wheel
(`C:\Python312\Lib\site-packages\clang\native`) on Windows. Set
`LIBCLANG_PATH` explicitly in CI.

See `docs/plans/2026-11-10-ffi-status.md` for the verification log, known
blockers and the `sllm-engine` integration plan.
