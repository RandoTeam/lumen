# sllm-llama-sys: FFI status (2026-11-10)

Scope: task "minimal but real `sllm-llama-sys`". The crate is standalone
(`[workspace]` in its `Cargo.toml`) and is intentionally absent from the root
workspace members list.

Pinned source: `external/llama-clover-src` at `4838ffb` ("speculative: add
masked-block self-speculative decoding for clover", PR #31306).

## Files

```text
crates/sllm-llama-sys/
  Cargo.toml              standalone package, links = "sllm_llama"
  build.rs                CMake + bindgen driver, host/Android, skip mode
  CMakeLists.txt          builds llama.cpp + the shim, installs static libs
  include/sllm_shim.h     C ABI of the shim
  include/sllm_bindgen.h  single bindgen input
  src/sllm_shim.cpp       extern "C" + try/catch + error codes
  src/lib.rs              public safe wrapper, Error types, platform cfg
  src/real.rs             bindgen-backed implementation
  src/stub.rs             implementation for SLLM_SYS_SKIP_BUILD=1
  examples/version.rs     version print + optional real model smoke test
  tests/version.rs        native build smoke tests
```

## Build flags

`build.rs` passes these to CMake (static by default):

| Option | Value |
|---|---|
| `BUILD_SHARED_LIBS` | `OFF` (`SLLM_SHARED=1` switches to shared) |
| `GGML_NATIVE` | `OFF` |
| `GGML_OPENMP` | `OFF` |
| `GGML_LLAMAFILE` | `OFF` |
| `GGML_CPU_KLEIDIAI` | `OFF` |
| `LLAMA_BUILD_COMMON` | `ON` |
| `LLAMA_BUILD_MTMD` | `ON` |
| `LLAMA_BUILD_TOOLS` | `OFF` |
| `LLAMA_BUILD_TESTS` | `OFF` |
| `LLAMA_BUILD_SERVER` | `OFF` |
| `LLAMA_BUILD_EXAMPLES` | `OFF` |
| `LLAMA_BUILD_APP` | `OFF` |
| `LLAMA_BUILD_UI` | `OFF` |
| `LLAMA_SUBPROCESS` | `OFF` |
| `LLAMA_CURL` | `OFF` |
| `LLAMA_OPENSSL` | `OFF` |
| `GGML_OPENCL` | only via `SLLM_OPENCL=1` or `--features opencl` |

The native side is always built as `CMAKE_BUILD_TYPE=Release` by default. Rust
never links the MSVC debug CRT (`/MDd`), so a Debug native build fails at link
time. Override with `SLLM_CMAKE_PROFILE` if needed.

## Host build (Windows, MSVC)

Environment used: Visual Studio Build Tools 2022 (MSVC 14.44), CMake 4.4.3,
Ninja, Rust 1.99.1, libclang from the Python `libclang` wheel.

```powershell
cd C:\OpenCode\SnapdragonLLM
$vcvars = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat'
$env:CARGO_TARGET_DIR = 'target/agent-ffi'
$env:LIBCLANG_PATH   = 'C:\Python312\Lib\site-packages\clang\native'
$env:SLLM_CMAKE_GENERATOR = 'Ninja'

cmd /c "call `"$vcvars`" >nul && cargo check --manifest-path crates\sllm-llama-sys\Cargo.toml --all-targets"
# result: Finished dev profile, exit 0

cmd /c "call `"$vcvars`" >nul && cargo test --manifest-path crates\sllm-llama-sys\Cargo.toml"
# result: 2 integration tests passed, 1 doctest passed
```

`cargo check` without a `vcvars` shell also works: CMake then selects the
Visual Studio generator (`Visual Studio 18 2026` on this host) and the build
script compiles everything. Only final linking (tests/examples) needs
`link.exe` on `PATH`, which is why the commands below use `vcvars64.bat`.
The check-only run took 3m04s in a clean target directory:

```powershell
$env:CARGO_TARGET_DIR = 'target/agent-ffi-plain'
$env:LIBCLANG_PATH = 'C:\Python312\Lib\site-packages\clang\native'
cargo check --manifest-path crates\sllm-llama-sys\Cargo.toml
# result: Finished dev profile, exit 0 (generator: Visual Studio 18 2026)
```

Smoke test with the local tiny model:

```powershell
$env:SLLM_TEST_MODEL = 'C:\OpenCode\SnapdragonLLM\models\tiny\Qwen2.5-0.5B-Instruct-Q4_K_M.gguf'
cmd /c "call `"$vcvars`" >nul && cargo run --manifest-path crates\sllm-llama-sys\Cargo.toml --example version"
```

Observed output (trimmed):

```text
llama_version = 0.7.0-dev
platform = host (android = false)
model description = qwen2 1B Q4_K - Medium
loaded in 301.15ms
n_ctx = 512, n_vocab = 151936
tokens (1): [9707]
decode took 67.57ms
logits = 151936
greedy next token = 271 ("\n\n")
detokenized prompt = "Hello"
```

(The description string comes from llama.cpp metadata; the file is the 0.5B
Qwen2.5 Q4_K_M model named in `docs/04-MODELS.md`.)

## Android build (arm64-v8a)

Environment: NDK `28.2.13676358`, min SDK 28, `c++_shared` (matches Rust std),
CPU baseline (`armv8-a`) by default.

```powershell
cd C:\OpenCode\SnapdragonLLM
$env:ANDROID_NDK_HOME = "$env:LOCALAPPDATA\Android\Sdk\ndk\28.2.13676358"
$env:CARGO_TARGET_DIR = 'target/agent-ffi'
$env:LIBCLANG_PATH   = 'C:\Python312\Lib\site-packages\clang\native'
$env:SLLM_CMAKE_GENERATOR = 'Ninja'
$env:SLLM_CPU_VARIANT = 'baseline'   # v82 | v86 | v90 for newer SoCs

cargo build --manifest-path crates/sllm-llama-sys/Cargo.toml --target aarch64-linux-android
# result: Finished dev profile, exit 0
# artifact: target/agent-ffi/aarch64-linux-android/debug/libsllm_llama_sys.rlib
```

Verified native libraries (installed by the build script):

```text
.../out/lib/libllama.a
.../out/lib/libllama-common.a
.../out/lib/libllama-common-base.a
.../out/lib/libmtmd.a
.../out/lib/libggml.a
.../out/lib/libggml-base.a
.../out/lib/libggml-cpu.a
.../out/lib/libvendor-hash.a
.../out/lib/libsllm_ffi.a
```

Architecture check:

```powershell
& "$env:ANDROID_NDK_HOME\toolchains\llvm\prebuilt\windows-x86_64\bin\llvm-objdump.exe" -f `
  target\agent-ffi\aarch64-linux-android\debug\build\sllm-llama-sys-*\out\lib\libllama.a |
  Select-String 'file format'
# elf64-littleaarch64
```

The crate produces an `rlib`; it does not link a final `.so` by itself, and the
smoke test was not executed on a device (no adb in this task). A future JNI
crate must:

1. link with the NDK `aarch64-linux-android28-clang` wrapper
   (`CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER`), and
2. pass `-Wl,-z,max-page-size=16384` (16 KB page devices), and
3. package `libc++_shared.so` once per APK, matching `ANDROID_STL=c++_shared`.

## Skip mode

```powershell
$env:SLLM_SYS_SKIP_BUILD = '1'
$env:CARGO_TARGET_DIR = 'target/agent-ffi'
cargo test --manifest-path crates/sllm-llama-sys/Cargo.toml
# result: exit 0; version() returns "unavailable (built with SLLM_SYS_SKIP_BUILD=1)"
#         and every fallible call returns ErrorKind::NotBuilt
```

This mode is for API/CI checks on machines without CMake/MSVC/NDK; it does not
pretend to be a working inference build.

## Errors hit while wiring this up, and fixes

1. **`llama_model_params` has no `use_mmap` / `use_mlock` in this fork.**
   The pinned PR replaced them with `enum llama_load_mode`. The shim and
   `ModelOptions` now expose `load_mode` (`LOAD_MODE_AUTO/MMAP/...`).
2. **`llama_batch_init()` returns `n_tokens = 0`.**
   `llama_decode` rejected the batch with `decode: n_tokens == 0`. The shim
   sets `batch.n_tokens = n_tokens` before filling the arrays.
3. **MSVC debug CRT mismatch.**
   `CMAKE_BUILD_TYPE=Debug` linked llama.cpp against `/MDd` while Rust used
   `/MD`, producing `__imp__CrtDbgReport` / `_calloc_dbg` link errors. Native
   builds are now Release by default (`SLLM_CMAKE_PROFILE`).
4. **NDK + `cmake` crate target override.**
   The `cmake` crate injected `CMAKE_C_FLAGS=--target=aarch64-linux-android`
   from its host-side `cc` probe, overriding the NDK toolchain's versioned
   `aarch64-none-linux-android28` target. Linker then failed to find
   `crtbegin_dynamic.o`. The build script now defines
   `CMAKE_C_FLAGS` / `CMAKE_CXX_FLAGS` / `CMAKE_ASM_FLAGS` as empty for Android
   so the NDK toolchain owns the flags. It also no longer passes
   `CMAKE_SYSTEM_NAME` / `CMAKE_ANDROID_NDK` / `ANDROID_NDK`.
5. **bindgen could not find clang builtin headers for Android.**
   The Python `libclang` wheel ships only `libclang.dll`, no `stdarg.h`.
   bindgen now gets `-resource-dir=<NDK>/toolchains/llvm/prebuilt/<host>/lib/clang/19`.
6. **Private static dependencies were not installed.**
   `llama-common-base` and `vendor-hash` were missing from `out/lib`, so a
   downstream link would fail. The crate's `CMakeLists.txt` installs them.
7. **libclang is not installed system-wide on this host.**
   `LIBCLANG_PATH` was unset and no `libclang.dll` existed under
   `C:\Program Files`. `build.rs` auto-detects the Python wheel location and
   prints `bindgen: using libclang at ...`; explicit `LIBCLANG_PATH` wins.

## What is verified vs not

Verified:

- host `cargo check --all-targets`, `cargo test`, `cargo clippy --all-targets
  -- -D warnings` with CMake/MSVC;
- `example version` loads the 397 MB Qwen2.5-0.5B Q4_K_M GGUF, tokenizes,
  decodes, samples and detokenizes;
- Android `cargo build --target aarch64-linux-android` with NDK 28, producing
  ARM64 (`elf64-littleaarch64`) llama/ggml/mtmd/static libs and an rlib.

Not verified:

- final Android `.so`/APK link and on-device execution (no adb per task);
- OpenCL backend build (flag exists, not exercised);
- `mtmd`, LoRA, `llama_batch_ext`/`llama_process`, threadpool and GPU offload
  are not exposed by the safe Rust API yet;
- performance numbers on a phone (host numbers above are desktop debug-Rust
  and only prove correctness).

## Next step: integrate with `sllm-engine`

1. Add a path dependency (do not add the crate to the root workspace members
   yet, and do not edit the root `Cargo.toml` in this task):

   ```toml
   # crates/sllm-engine/Cargo.toml
   [dependencies]
   sllm-llama-sys = { path = "../sllm-llama-sys" }
   ```

2. Wrap this API in `sllm-engine` behind a `LlamaEngine` implementing the
   existing `Engine::load/generate/stream/trim` contract:
   - `backend_init()` once at process start (or a `OnceLock`);
   - `ModelOptions { n_gpu_layers: 0, load_mode: LOAD_MODE_MMAP }` for the
     845/8 Gen 3 CPU baseline, `LOAD_MODE_MMAP_MLOCK` only behind a setting;
   - `ContextOptions` mapping `n_ctx`, `n_batch`, `n_ubatch`, thread counts and
     `type_k/type_v` (q8_0 = `GGML_TYPE_Q8_0`) from `ModelPack`;
   - sampling via `SamplerConfig`; `Sampler::sample` already accepts the token
     it returns, so use `Sampler::accept` only for externally chosen tokens
     (forced tool tokens, injected prefixes);
   - grow the loop with `decode(tokens, start_pos, seq, false)` then
     `sample()`; reuse `start_pos` instead of re-decoding the prompt;
   - keep `Context` and `Sampler` on the same thread (they are `Send`, not
     `Sync`) or move them together behind a mutex in the JNI/engine owner.
3. Add a CI/Android job that runs the two commands in this document with
   `SLLM_CPU_VARIANT=v86` and the `target/agent-ffi` cache restored.
4. Before shipping, pin `LIBCLANG_PATH` in the build image (the Python wheel
   path is a convenience, not a contract) and decide whether `mtmd` stays in
   the static link or becomes an optional feature.

## Reproduce from scratch

```powershell
Remove-Item -Recurse -Force target\agent-ffi
cd C:\OpenCode\SnapdragonLLM
$env:CARGO_TARGET_DIR = 'target/agent-ffi'
$env:LIBCLANG_PATH   = 'C:\Python312\Lib\site-packages\clang\native'
$env:SLLM_CMAKE_GENERATOR = 'Ninja'
& 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat'
cargo check --manifest-path crates\sllm-llama-sys\Cargo.toml --all-targets
```

Full logs from this session are under `C:\Temp\opencode\sllm_*.log` (host
check/test/example and Android build logs).
