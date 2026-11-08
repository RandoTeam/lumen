# Отчёт 10. Rust ↔ llama.cpp: FFI и матрица сборок

Источник: исследование архитектуры, 2026-11-08. Дерево: `external/llama-clover-src`
(4838ffb), mainline `external/llama.cpp`. Устройства: Pad 2 (8 Gen 3, 16 GB),
MI 8 (845, 6 GB).

## TL;DR

Строим **своё `sllm-llama-sys`**: `bindgen` по C-заголовкам + тонкий C++ shim,
который компилируется вместе с `external/llama-clover-src` через CMake.
`llama-cpp-2`/`llama-cpp-sys-2` не подходят для Clover: они везут свой submodule
`llama.cpp`, а bindings у них предгенерированы; `llama_process`/`llama_batch_ext`
и полные `common_chat_templates` там не обёрнуты. Патчить crate целиком дороже,
чем сделать свой sys-крейт.

Ядро — `sllm-core` с `Engine::load/generate/stream`, `ModelPack`,
`MemoryGovernor`, `Backend`. Clover собираем из `4838ffb` (PR #31306), остальное —
из того же дерева; mainline-флейвор оставляем для моделей/типов после 4838ffb.
845 — только CPU baseline; 8 Gen 3/Elite — CPU v8.6, затем opt-in OpenCL,
Vulkan/Hexagon — после замеров.

## 1. Решение по FFI

| Вариант | Плюсы | Минусы для нас |
|---|---|---|
| `llama-cpp-2` 0.1.161 | безопасный API, LoRA, mtmd, `GgufContext`, sampling | свой `llama.cpp`; предгенерированные bindings; нет `llama_process`/`batch_ext`; chat templates только строкой |
| `llama-cpp-sys-2` 0.1.161 | raw bindings, хороший `build.rs` (Android NDK, `GGML_CPU_ARM_ARCH`, `GGML_*`, dynamic backends) | нельзя подменить исходник; bindings уже сгенерированы; `[patch.crates-io]` = форк всего крейта |
| `llama-cpp-4` 0.11 | mtmd/prebuilt/opencl, форк | тот же vendored-source, меньше контроля |
| `llama_cpp` 0.3.2 | простой | мёртв |
| **свой `bindgen` + CMake + shim** | pinned `4838ffb`, любые символы, `llama_process`, `common_*`, mtmd, контроль сборки | ~500 строк shim и обвязки |

**Выбор:** свой sys-крейт. C++ exceptions не должны пересекать FFI: все входные
функции shim — `extern "C"` и внутри `try/catch(...)`, возвращают `int` + текст
ошибки. Rust-паника не должна идти в C++: на FFI-границе `catch_unwind`.

Ключевые заголовки: `include/llama.h`, `ggml/include/ggml.h`,
`ggml-backend.h`, `ggml-cpu.h`, `gguf.h`, `tools/mtmd/mtmd.h`; C++-часть —
`common/chat.h`, `common/common.h`, `common/sampling.h`. Символы:
`llama_model_load_from_file/splits/_from_file_ptr`, `llama_init_from_model`,
`llama_batch_ext_*`, `llama_process`, `llama_sampler_*`,
`llama_adapter_lora_*`, `llama_set_adapters_lora`, `llama_memory_*`,
`llama_state_get_size`, `mtmd_*`, `ggml_backend_load_all_from_path`,
`ggml_backend_dev_*`, `ggml_threadpool_*`, `llama_perf_context`.

`common_batch` — C++ RAII над `llama_batch_ext`; ABI нестабилен. В Rust делаем
свой `Batch` с теми же операциями (`add_token`, `add_seq`, `set_pos`,
`set_output_logits`) и проверяем на тестах `common_batch`.
`llama_process(LLAMA_PROCESS_TYPE_DECODE, batch_ext)` используем вместо
`llama_decode`.

## 2. Cargo/CMake рецепт

Workspace:

```toml
[workspace]
members = ["crates/sllm-llama-sys", "crates/sllm-gguf", "crates/sllm-core",
           "crates/sllm-jni", "crates/sllm-cli", "crates/sllm-bench"]
resolver = "3"

[workspace.package]
edition = "2024"
rust-version = "1.99"
license = "Apache-2.0"

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
panic = "unwind"
strip = "symbols"
```

`crates/sllm-llama-sys/Cargo.toml`:

```toml
[package]
name = "sllm-llama-sys"
version = "0.1.0"
edition.workspace = true
links = "sllm_llama"
build = "build.rs"

[build-dependencies]
cmake = "0.1"
bindgen = "0.73"

[features]
default = ["clover"]
clover = []
mainline = []
opencl = []
vulkan = []
hexagon = []
dynamic-backends = []
```

`build.rs` (каркас):

```rust
use std::{env, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = PathBuf::from(env::var("SLLM_LLAMA_SRC")
        .unwrap_or_else(|_| root.join("../../external/llama-clover-src").display().to_string()));
    let ndk = env::var("ANDROID_NDK_HOME").expect("ANDROID_NDK_HOME");
    let abi = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64-v8a", "arm" => "armeabi-v7a", x => panic!("{x}"),
    };
    let variant = env::var("SLLM_CPU_VARIANT").unwrap_or_else(|_| "baseline".into());
    let arch = match variant.as_str() {
        "baseline" => "armv8-a",
        "v82" => "armv8.2-a+dotprod+fp16",
        "v86" => "armv8.6-a+dotprod+i8mm+fp16",
        "v90" => "armv9.2-a+dotprod+i8mm+fp16+sve2",
        _ => panic!("unknown SLLM_CPU_VARIANT"),
    };
    let dynamic = env::var("SLLM_DYNAMIC_BACKENDS").is_ok();

    let mut cfg = cmake::Config::new("cmake");
    cfg.define("SLLM_LLAMA_SRC", &src)
       .define("CMAKE_TOOLCHAIN_FILE", format!("{ndk}/build/cmake/android.toolchain.cmake"))
       .define("ANDROID_ABI", abi)
       .define("ANDROID_PLATFORM", "android-28")
       .define("ANDROID_STL", "c++_static")
       .define("BUILD_SHARED_LIBS", if dynamic { "ON" } else { "OFF" })
       .define("GGML_NATIVE", "OFF")
       .define("GGML_OPENMP", "OFF")
       .define("GGML_LLAMAFILE", "OFF")
       .define("GGML_CPU_KLEIDIAI", "OFF")
       .define("GGML_CPU_REPACK", if variant == "baseline" { "OFF" } else { "ON" })
       .define("GGML_BACKEND_DL", if dynamic { "ON" } else { "OFF" })
       .define("GGML_CPU_ALL_VARIANTS", if dynamic { "ON" } else { "OFF" })
       .define("GGML_CPU_ARM_ARCH", if dynamic { "" } else { arch })
       .define("LLAMA_BUILD_COMMON", "ON")
       .define("LLAMA_BUILD_MTMD", "ON")
       .define("LLAMA_BUILD_TOOLS", "OFF")
       .define("LLAMA_BUILD_TESTS", "OFF")
       .define("LLAMA_BUILD_SERVER", "OFF")
       .define("LLAMA_CURL", "OFF")
       .define("LLAMA_OPENSSL", "OFF");
    if env::var("SLLM_OPENCL").is_ok() {
        cfg.define("GGML_OPENCL", "ON")
           .define("GGML_OPENCL_USE_ADRENO_KERNELS", "ON");
        if let Ok(p) = env::var("OPENCL_INCLUDE_DIR") {
            cfg.define("OpenCL_INCLUDE_DIR", p);
        }
    }
    let out = cfg.build();
    println!("cargo:rustc-link-search=native={}", out.join("lib").display());
    // static: start-group of sllm_ffi/mtmd/llama-common/llama/ggml*
    // dynamic: libsllm_ffi.so + libggml-cpu-*.so, load via nativeLibraryDir
}
```

`cmake/CMakeLists.txt`:

```cmake
cmake_minimum_required(VERSION 3.22)
project(sllm_ffi C CXX)
set(CMAKE_CXX_STANDARD 17)
set(LLAMA_BUILD_COMMON ON CACHE BOOL "" FORCE)
set(LLAMA_BUILD_TOOLS OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_MTMD ON CACHE BOOL "" FORCE)
add_subdirectory(${SLLM_LLAMA_SRC} ${CMAKE_BINARY_DIR}/llama EXCLUDE_FROM_ALL)

add_library(sllm_ffi STATIC src/sllm_shim.cpp)
target_include_directories(sllm_ffi PUBLIC
  ${SLLM_LLAMA_SRC}/include ${SLLM_LLAMA_SRC}/ggml/include
  ${SLLM_LLAMA_SRC}/common ${SLLM_LLAMA_SRC}/tools/mtmd)
target_link_libraries(sllm_ffi PUBLIC mtmd llama-common llama ggml)
target_compile_options(sllm_ffi PRIVATE -fvisibility=hidden -fexceptions -frtti)
```

Shim guard:

```cpp
template <class F>
static int sllm_guard(char *err, size_t n, F &&f) noexcept {
    try { return f(); }
    catch (const std::bad_alloc &) { std::snprintf(err, n, "OOM"); return -2; }
    catch (const std::exception &e) { std::snprintf(err, n, "%s", e.what()); return -3; }
    catch (...) { std::snprintf(err, n, "unknown C++ exception"); return -4; }
}
```

Сборка:

```powershell
$env:ANDROID_NDK_HOME="$env:LOCALAPPDATA\Android\Sdk\ndk\28.2.13676358"
$env:SLLM_CPU_VARIANT="v86"
cargo ndk -t arm64-v8a build --release -p sllm-jni
```

16 KB pages: `-Wl,-z,max-page-size=16384`; проверять `readelf -lW`.

**Статика vs динамика.** Статика: `BUILD_SHARED_LIBS=OFF`, `GGML_BACKEND_DL=OFF`,
один `libsllm_jni.so`, `c++_static`, `--exclude-libs,ALL`; проще, но нет
universal CPU variants. Динамика: `BUILD_SHARED_LIBS=ON`, `GGML_BACKEND_DL=ON`,
`GGML_CPU_ALL_VARIANTS=ON`; появляются `libggml-cpu-android_*.so`, `libllama.so`,
`libmtmd.so`; грузить через `ggml_backend_load_all_from_path(nativeLibraryDir)`.
CMake запрещает `ALL_VARIANTS` без `BACKEND_DL`, а `BACKEND_DL` без shared.

## 3. Матрица сборок

| Флейвор | Устройства | `GGML_CPU_ARM_ARCH` | Особенности | Оценка |
|---|---|---|---|---|
| baseline | 845, старые | `armv8-a` | repack off, без KleidiAI | 18–25 MB |
| v82 | 855/865/778G | `armv8.2-a+dotprod+fp16` | repack Q4_0 | 20–26 MB |
| v86 | 8 Gen 3, 8 Elite | `armv8.6-a+dotprod+i8mm+fp16` | repack, KleidiAI опц. | 22–28 MB |
| v90 | 8 Elite Gen 5 / X Elite | `armv9.2-a+dotprod+i8mm+fp16+sve2` | только при HWCAP2_SVE2 | 24–30 MB |
| universal | все | все варианты | ALL_VARIANTS + BACKEND_DL | +50–80 MB |

**Не ставить `-C target-feature=+dotprod,+i8mm` на весь Rust-крейт.** Rust
собираем baseline, `-march` — только C++-цели через `GGML_CPU_ARM_ARCH`. Иначе
std/alloc упадут на 845.

`GGML_CPU_ALL_VARIANTS` собирает `android_armv8.0_1`, `android_armv8.2_1`,
`android_armv8.2_2`, `android_armv8.6_1`, `android_armv9.0_1`,
`android_armv9.2_1/2`; runtime выбирает по `ggml_backend_score`. 845 — baseline,
8 Gen 3 — v8.6, 8 Elite — v8.6/v9.2 при SVE2.

Рекомендация: v1 — два APK (baseline, v86); v2 — universal для sideload.
Play AAB не умеет CPU-feature splits.

## 4. GPU/NPU

| Бэкенд | 845 / Adreno 630 | 8 Gen 3 / Adreno 750 | 8 Elite / Adreno 830 |
|---|---|---|---|
| Vulkan | **не работает** (16-bit storage; патч → SIGSEGV) | драйвер падает (#29165), FA (#30391) | vendor abort (#29786); Turnip + баги (#28635, #30654) |
| OpenCL | 3.05/2.23 — в 12–14× медленнее CPU | 1.5B Q4_0: 252 pp / 24.1 tg; 3B: 136/21.4 | ~199–275 pp, 22 tg |
| Hexagon | v65 нет; QNN/SNPE — тупик | v75: Q4_0/Q8_0/MXFP4, MoE, 3.5 GB/сессия | v79: Llama-1B 169/51.5; GPT-OSS-20B на 4 сессиях |

OpenCL Adreno-ядра (`gemm_*_q4_k_q8_1_dp4a`, MoE, GDN, FA) на A6X отключаются —
отсюда провал 845. На A7X/A8X включать `GGML_OPENCL_USE_ADRENO_KERNELS=ON`;
сначала Q4_0/Q6_K, затем Q4_K. Vulkan на 750/830 — не по умолчанию, только
allowlist + probe-процесс. Hexagon: `GGML_HEXAGON=ON`, Docker
`ghcr.io/snapdragon-toolchain/arm64-android:v0.8`; для Clover скорее нет, для
dense/Qwen/Bonsai — после CPU-базлайна.

Backend — runtime-параметр:

```rust
pub enum Backend {
    Cpu { variant: CpuVariant },
    OpenCl { device: String },
    Vulkan { device: String },
    Hexagon { sessions: u8 },
}
```

`Engine::load` перечисляет `ggml_backend_dev_count/get_props`, фильтрует,
ставит `n_gpu_layers`, `offload_kqv`, `op_offload`, `moe_cache_size`.
`auto`: 845 → CPU; 8 Gen 3/Elite → CPU по умолчанию, OpenCL opt-in для prefill;
Vulkan/Hexagon — только если `.so` есть и модель в allowlist. Ошибка → CPU
fallback + лог.

## 5. Модели, LoRA, mmproj

`ModelPack`:

```json
{
  "engine": "clover",
  "base": ["base.gguf"],
  "mmproj": null,
  "adapters": [{"path": "adapters/uncensored.gguf", "scale": 1.0, "enabled": true}],
  "ctx": 4096, "kv": "q8_0"
}
```

Preflight: magic/version, `general.architecture`, `general.type`
(model/adapter/mmproj), `GGML_TYPE_COUNT`, типы тензоров, split-шары, SHA-256,
chat template, `prism.hadamard.version`, метаданные адаптера. Для 4838ffb:
Q2_0 (42) читается, PTQ1_0/PQ2_0 (143/142) — нет; показывать
«формат не поддерживается этим билдом».

Оценка памяти:

```text
total ≈ file_bytes (mmap/PSS) + KV + recurrent_state + compute + lora + mmproj
KV = n_ctx × n_full_attn_layers × n_head_kv × (k_len + v_len) × bytes_per(kv)
```

Clover: 10 full-attention, `head_count_kv=2`, k/v=256 → KV q8_0 4096 ≈ 45 MB,
RS ≈ 63 MB, compute ≈ 127 MB. Bonsai: 8 full-attention, KV ≈ 68 MB, RS ≈ 50 MB.
Бюджет: `availMem` − reserve 1.5 GB (6 GB), 2.5 GB (16 GB), 4 GB (24 GB).

Clover — только 4838ffb. Bonsai — qwen35, Q2_0, hadamard v2; LoRA rank-1
2.37 MB, 64 сайта, runtime, +0–5% decode. LoRA API:
`llama_adapter_lora_init`, `llama_adapter_lora_init_from_file_ptr` (SAF),
`llama_set_adapters_lora(ctx, adapters, n, scales)`, `llama_adapter_meta_*`.
mmproj: `mtmd_init_from_file`, `mtmd_default_marker`, `mtmd_tokenize`,
`mtmd_helper_eval_chunks`.

## 6. Память и потоки

- Persistent threadpool: `ggml_threadpool_params_default(n)`,
  `ggml_threadpool_new`, `llama_attach_threadpool(ctx, pool, pool_batch)`.
  На 845 это дало 18→25 tok/s.
- Потоки: 845 — 4 big; 8 Gen 3 — 4–6; 8 Elite — 6–8; pin big cores по маске.
- KV q8_0 по умолчанию; q4_0 — только для длинного ctx/6 GB + quality gate.
- mmap: `MMAP`, не `MLOCK`; `MADV_WILLNEED` для prefetch, `DONTNEED` при trim.
- onTrimMemory: MODERATE — mtmd/bitmap cache; RUNNING_LOW — KV; COMPLETE —
  `llama_free`, оставить metadata; через `Engine::trim`.
- LMK: foreground service; PSS важнее RSS; file-backed mmap вытесняем.

## 7. Производительность

Факт только 845 (Qwen 0.5B: 26.6 tg / 41.8 pp); остальное — гипотезы.

| Модель | 845 6 GB | Pad 2 8 Gen 3 16 GB | 8 Elite 24 GB |
|---|---:|---:|---:|
| Qwen2.5-0.5B Q4_K_M | 26.6 / 41.8 | 40–65 / 150–250 | 50–80 / 200–300 |
| Bonsai 4B Q2_0 | 3–5 / 5–8 | 8–15 / 15–30 | 12–20 / 25–40 |
| Clover 35B-A3B | 0.6 / 2.4 | 2–6 / 5–12 | 5–10 / 10–20 |

Clover на 8 Gen 3 упирается в compute + чтение экспертов: ~350–420 MB активных
весов/токен; даже при 100% page cache потолок ~6–10 tok/s, при misses 2–4.
Мерить `llama_perf_context`, TTFT, `/proc/self/io`, majflt, meminfo, thermal,
top -H, Perfetto.

Оптимизации: persistent pool (+30–40%), CPU variant (+20–50% на 8 Gen 3),
repack Q4_0 (+10–25%), batch 128→256 для prefill, KV q8_0/q4_0, big-core pin.

## 8. План, риски, проверки

**Крейты:** `sllm-llama-sys` (bindgen+shim+CMake), `sllm-gguf` (preflight, split,
SHA), `sllm-core` (Engine, ModelPack, Sampling, MemoryGovernor, Backend),
`sllm-jni` (тонкий C ABI/JNI), `sllm-cli`, `sllm-bench`.

**Порядок:** (0) sys + shim, `llama_version`, `llama_process`, Qwen на host/Pad 2;
(1) gguf + ModelPack + blocking generate; (2) streaming, chat templates, sampler,
LoRA, JNI, trim; (3) Clover, batch_ext, mmap/IO; (4) CPU variants/universal,
OpenCL opt-in, Vulkan guard, Hexagon; (5) mtmd, split GGUF, benchmark/gate.

**Риски:** fork 4838ffb bitrot — vendored tarball + SHA + CI; C++ exceptions —
shim; duplicate libc++ — один `.so`, `c++_static`, `--exclude-libs,ALL`; APK
size — два флейвора; LMK — preflight + governor; Vulkan crash — allowlist/off;
Hexagon SDK/Docker — отдельный эксперимент; QNN/Clover — не тратить; 845 — не
включать dotprod/i8mm ни в Rust, ни в C++.

**Проверить:** `cpuinfo` (`asimddp`, `i8mm`, `bf16`, `sve2`), `ro.soc.model`,
`getconf PAGE_SIZE`, `readelf -lW` (0x4000), CPU baseline, OpenCL `-ngl 99`,
Clover ctx 2048/4096, KV q8_0/q4_0, mmap vs DIRECT_IO, 4/6/8 threads,
20–30 мин sustained.

Ссылки: PR #31306, llama-cpp-rs, llama-cpp-2/sys-2, cargo-ndk, bindgen,
Android 16 KB pages; issues #29165, #30391, #29786, #28635, #30654, #30649,
#27191, #30821, #27723; OpenCL #10693, #14841, #14987.
