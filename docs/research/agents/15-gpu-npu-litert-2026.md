# Отчёт 15. GPU/NPU/LiteRT: актуальный стек для локального инференса LLM (ноябрь 2026)

**Дата:** 2026-11-07  
**Область:** LiteRT-LM, llama.cpp backends, Qualcomm QNN/Hexagon/Genie, speculative decoding.  
**Устройства:** Xiaomi MI 8 (SDM845, Adreno 630, Hexagon 685, 6 GB); OnePlus Pad 2 (SM8650, Adreno 750, Hexagon v75, 16 GB); OnePlus 13 (SM8750, Adreno 830, Hexagon v79, 24 GB).  
**Статус:** исследование + аудит артефактов; на живых устройствах в рамках этого отчёта замеров **не делалось**. Все числа, полученные ранее проектом, помечены «факт»; оценки — «оценка»; отсутствие данных — «нет данных».

---

## 0. TL;DR

1. **LiteRT-LM — главный «официальный» путь для Gemma 4.** На ноябрь 2026 актуален **v0.19.1** (2026-11-05), Apache-2.0, Maven `com.google.ai.edge.litertlm:litertlm-android:0.19.1`. Поддерживает Gemma 4 E2B/E4B, Gemma 3n, Qwen, Phi-4, FunctionGemma; формат `.litertlm`; MTP-драфтер **внутри файла модели**; GPU через OpenCL/ML Drift, NPU через Qualcomm QAIRT dispatch.
2. **Gemma 4 E4B на SD845 (6 GB) — не про скорость, а про память.** Файл 3.66 GB (2.24 GB весов + 0.67 GB эмбеддингов), RSS на слабом CPU-классе ~3.1–3.3 GB. На 6 GB это на грани OOM/LMK. GGUF Q4_0 — 4.38–4.6 GB, то есть ещё хуже. Реалистичный Gemma для 845 — **E2B (2.58 GB)**, не E4B.
3. **На 845 GPU и NPU мертвы для LLM.** OpenCL: 3.05 pp / 2.23 tg на Qwen2.5-0.5B против 41.8 / 26.6 на CPU (факт). Vulkan: устройство видно, но `16-bit storage` → отказ; патч → SIGSEGV (факт). Hexagon 685 = v65, а `ggml-hexagon`/QAIRT начинаются с v68/v73. Остаётся CPU.
4. **На 8 Gen 3 (Pad 2, 16 GB) рабочий путь — CPU, затем OpenCL.** LiteRT GPU на Adreno 750 имеет подтверждённый баг: порча текста/цифр на промптах ~2k токенов (issue #3012, тот же SoC/Android 16). NPU для Gemma 4 **E4B** на SM8650 официального bundle нет; для **E2B** NPU-bundle, судя по issue #4183, существует (но audio там падает на CPU), а для Gemma3-1B есть `sm8650`.
5. **На 8 Elite (OnePlus 13, 24 GB) появляется смысл в Clover 35B-A3B на CPU** (11.69 GB влезает) и в Gemma 4 E4B GPU/MTP. NPU через QAIRT/GenieX возможен, но **доступ из обычного APK зависит от прошивки** (unsigned PD / FastRPC).
6. **llama.cpp 2026 — это CPU + OpenCL для Adreno + Hexagon backend.** OpenCL официально проверен на Adreno 750/810/830/840/X1/X2; A6x в телефонах «likely not supported» из-за старого драйвера. Vulkan на Adreno 750/830 имеет известные падения/неверные результаты и не рекомендуется.
7. **KleidiAI** (`GGML_CPU_KLEIDIAI=ON`) даёт ускорение Q4_0/Q4_K/Q6_K/Q8_0 на ARM64 с dotprod/i8mm, но **не помогает 845** (там нет dotprod/i8mm) и не покрывает Q2_0.
8. **Speculative decoding в llama.cpp уже есть**: `draft-simple`, `draft-mtp`, `draft-eagle3`, `draft-dflash`, `draft-dspark`, `draft-self`, `ngram-*`. Для Gemma 4 E4B опубликован MTP-драфтер 57 MB (Q4_0). Для Clover в форке 4838ffb есть `draft-self` (masked-block). Для 845 практичен только `ngram-*`.
9. **MTP — не «3x на телефоне».** Официальные замеры LiteRT на S26 Ultra: CPU 17.0 → 27.5 tg/s (summarize), GPU 21.9 → 46.0 tg/s. На 845 MTP бессмысленен из-за памяти и compute; на 8 Gen 3/Elite — да, но только при высоком acceptance и с оговорками.
10. **Rust-линковка возможна.** LiteRT-LM C API 1.x (`litert_lm_*`) стабилен по ABI, но **Maven AAR содержит только JNI и не экспортирует C-символы** — нужно собирать `//c:litert-lm` из исходников Bazel/NDK. Альтернатива — GenieX C SDK (`geniex_*`, BSD-3, AAR 89 MB zip / ~220 MB jni), который уже отдаёт GGUF + CPU/OpenCL/Hexagon + QAIRT.
11. **Главный риск NPU — не производительность, а доступ.** На retail-прошивках unsigned PD может быть запрещён: `ggml-hex` пишет `failed to enable unsigned PD`, QNN — `createUnsignedPD ... not supported by HTP`. Shell/`adb` работает, обычный app UID — нет. Signed PD требует вендорной подписи. Это надо проверять на каждом устройстве.
12. **16 KB pages:** LiteRT-LM, GenieX и ARM-части QNN уже выровнены на `0x4000`; Hexagon-skel — 32-битный ELF с `0x1000`, это нормально для DSP, но требует отдельной проверки упаковки/Play. `libc++_shared` ни одному из трёх prebuilt-стека не нужен (statically linked).

---

## 1. Устройства и границы применимости

| | MI 8 (dipper) | OnePlus Pad 2 (OPD2403) | OnePlus 13 |
|---|---|---|---|
| SoC | Snapdragon 845 (SDM845) | Snapdragon 8 Gen 3 (SM8650) | Snapdragon 8 Elite (SM8750) |
| CPU | 4×Cortex-A75 @2.8 + 4×A55 @1.8 | 1×X4 + 3×A720 + 2×A720 + 2×A520 | 2×Oryon + 6×Oryon |
| GPU | Adreno 630 | Adreno 750 | Adreno 830 |
| NPU/DSP | Hexagon 685 (**v65**) | Hexagon (**v75**) | Hexagon (**v79**) |
| RAM | 6 GB LPDDR4X | 16 GB LPDDR5X | 24 GB LPDDR5X |
| GPU API | OpenCL 2.0, Vulkan 1.1, GLES 3.2 | OpenCL 3.0, Vulkan 1.3 | OpenCL 3.0, Vulkan 1.3 |
| Android в проекте | 15 (custom ROM) | 16 | 15/16 (OxygenOS) |
| ARM-фичи | ARMv8.2-A, **нет dotprod/i8mm** | ARMv9.2, dotprod+i8mm | ARMv9.2 Oryon, dotprod+i8mm |

**Важно про «8 Elite».** В проекте это OnePlus 13 на **SM8750**: Adreno 830, Hexagon **v79**, 24 GB. Не путать с **Snapdragon 8 Elite Gen 5 (SM8850)**: Adreno 840, Hexagon **v81** — именно на нём сделаны многие свежие замеры LiteRT (S26 Ultra) и issue #4001. Числа с S26 Ultra нельзя напрямую переносить на OnePlus 13.

**Легенда матриц:**

| Символ | Значение |
|---|---|
| ✅ | проверено/поддерживается, есть измерение |
| 🟡 | ожидаемо работает, нужен замер |
| 🟠 | работает частично / рискованно / есть известный баг |
| ❌ | не поддерживается или бессмысленно |
| 💀 | не влезает в память / практически невозможно |
| ❓ | данных нет |

---

## 2. LiteRT-LM (Google AI Edge) в ноябре 2026

### 2.1. Статус, версии, артефакты

| Параметр | Значение |
|---|---|
| Репозиторий | https://github.com/google-ai-edge/LiteRT-LM |
| Последний релиз | **v0.19.1**, 2026-11-05 |
| Лицензия runtime | **Apache-2.0** |
| Maven (Android) | `com.google.ai.edge.litertlm:litertlm-android:0.19.1` |
| Maven (JVM) | `com.google.ai.edge.litertlm:litertlm-jvm` |
| AAR размер | **21 200 360 B (~21.2 MB)**; `jni/arm64-v8a/liblitertlm_jni.so` — **22 692 856 B (~21.6 MB)** |
| minSdk AAR | **24** |
| ABI | `arm64-v8a`, `x86_64` (в AAR); GPU prebuilt — только arm64 |
| NDK для сборки из исходников | **r28b+** (в docs), Bazel 7.6.1 |
| C API | 1.x; в v0.19.0 объявлен **1.0.0 (breaking)**, в заголовках main — **1.1.0**; ABI проверяется `litert_lm_get_api_version` |
| Prebuilt в репо | `prebuilt/android_arm64/libGemmaModelConstraintProvider.so`, `libwebgpu_dawn.so` (GPU-библиотеки приходят из LiteRT/Maven, не из этого каталога) |

Что нового в 0.19.x (из релизных notes):

- v0.19.1: multimodal Gemma 4 (vision + audio) на **Artisan GPU** backend; bidirectional attention для vision; audio encoders с Artisan GPU.
- v0.19.0: accelerated logit masking (в т.ч. для MTP), Web SDK, ASR/TTS (Qwen3-ASR, Kokoro, Qwen3-TTS), **C API 1.0.0 breaking**, GPU sampling == CPU для одного seed.
- v0.18.0: EmbeddingGemma 2, ModelInfo API, dynamic KV growth на NPU, pruning attention mask на GPU.

### 2.2. Формат `.litertlm` и поддерживаемые модели

`.litertlm` — единый контейнер: фиксированный бинарный префикс (magic + SemVer) → FlatBuffer-заголовок с индексом секций → сами секции (TFLite-модель/веса, tokenizer, metadata, при наличии — MTP-драфтер). Это развитие `.task`; для сборки/инспекции есть Python-пакет `litert-lm-builder` (CLI build/inspect/unpack), см. https://developers.google.com/edge/litert-lm/file_builder.

Официальные модели (litert-community на Hugging Face):

| Модель | Размер | Контекст | Квантование | Комментарий |
|---|---:|---:|---|---|
| **Gemma4-E4B** | **3.66 GB** (текст: 2.24 GB weights + 0.67 GB embeddings; остальное vision/audio) | 32k | per-channel int4 (по карточкам E2B/NPU и issue #4185) | Основная цель; MTP-драфтер внутри файла |
| Gemma4-E2B | 2.58 GB | 32k | per-channel int4 | Реалистичный вариант для 6 GB |
| Gemma3n-E4B | 4.69 GB | 32k | int4 | Старое поколение; S24 Ultra: 74/9 CPU, 548/9 GPU |
| Gemma3n-E2B | 2.97 GB | 32k | int4 | |
| Gemma3-1B | 557 MB (int4 `.litertlm`); 1.0 GB int8 `.task` | 1.28k–4k | int4/int8 | NPU-варианты `sm8550/sm8650/sm8750/sm8850` — 657–662 MB |
| Qwen2.5-0.5B | 521 MB | 1.28k | **dynamic int8** (int8 weights, float activations) | Официального `.litertlm` в текущем HF-репо **нет** — только `.task`/`.tflite`; есть community `.litertlm` |
| Qwen2.5-1.5B | 1.52 GB | 4k | int8 (q8 `.litertlm`) | S25 Ultra: 298/34 CPU, 1668/31 GPU |
| Qwen3-0.6B | 586 MB | — | int4 | |
| Phi-4-mini | 3.9 GB | — | int4 | S24 Ultra: 67/7 CPU, 314/10 GPU |
| FunctionGemma | 289 MB | — | int4 | Tool calling |

**Терминология.** «Gemma 3 E4B» не существует: у Gemma 3 — 1B/4B/12B/27B, а E2B/E4B — это Gemma 3n и Gemma 4. В отчёте везде, где важно, указано поколение; не смешивать Gemma 3n E4B (4.69 GB) и Gemma 4 E4B (3.66 GB).

**Варианты файла Gemma 4 E4B (факт по HF tree):**

| Файл | Размер | Назначение |
|---|---:|---|
| `gemma-4-E4B-it.litertlm` | 3.66 GB (3490 MiB) | универсальный CPU/GPU + multimodal |
| `gemma-4-E4B-it-gpu.litertlm` | 2.97 GB (2832 MiB) | GPU-оптимизированный |
| `gemma-4-E4B-it-web.litertlm` | 2.97 GB | Web/WebGPU, text-only |
| `gemma-4-E4B-it-web.task` | 2.96 GB | legacy MediaPipe Web |
| `gemma-4-E4B-it_qualcomm_sm8750.litertlm` | — | **загружен и удалён 2026-10-19** (вместе с `sm8850` и `qcs8275`); в текущем tree отсутствует; не обещать |

Отдельно: в issue #2226 фигурирует `gemma-4-E2B-it_qualcomm_sm8750.litertlm` — то есть NPU-вариант **E2B** для SM8750 существует/существовал, а E4B — под вопросом.

### 2.3. API

| Язык | Статус | Как использовать |
|---|---|---|
| **Kotlin/Java** | ✅ stable | `EngineConfig(modelPath, backend = Backend.CPU()/GPU()/NPU(nativeLibraryDir))`, `Engine.initialize()`, `createConversation()`, `sendMessageAsync(): Flow<Message>` |
| **C++** | ✅ stable | `Engine::CreateEngine(EngineSettings)`, `Conversation`, `Message` (nlohmann ordered_json), `SendMessageAsync` |
| **C** | ✅ stable (1.x) | `litert_lm_engine_settings_create`, `litert_lm_engine_create`, `litert_lm_session_*`, `litert_lm_conversation_*`, `litert_lm_speculative_decoding_config_*` |
| **Python** | ✅ stable | `litert_lm.Engine(...)` |
| **Swift / JS / Flutter** | preview/community | iOS/macOS, Web, Flutter |
| **Rust** | ❌ официального crate нет | В репо есть только `rust/BUILD` (аллокатор/настройки), публичного Rust API нет. Через C ABI — да (см. 2.8) |

Ключевые возможности Kotlin API:

- `EngineConfig(modelPath, backend, visionBackend, audioBackend, cacheDir, maxNumTokens, activationDataType)`.
- `ConversationConfig(systemInstruction, initialMessages, samplerConfig, tools, thinkingConfig, maxOutputToken)`.
- `ExperimentalFlags.enableSpeculativeDecoding = true` (MTP).
- `ThinkingConfig(enableThinking, thinkingTokenBudget)`.
- Tool calling: `ToolSet` + `@Tool`/`@ToolParam` или `OpenApiTool`.
- `Message.channels["thought"]` для reasoning-токенов.

C API (важное для Rust):

- `litert_lm_engine_settings_create(model_path, backend_str, vision_backend, audio_backend, &settings)`.
- `litert_lm_engine_settings_set_num_threads`, `..._set_max_num_tokens`, `..._set_cache_dir`, `..._set_litert_dispatch_lib_dir` (NPU).
- `litert_lm_engine_settings_set_speculative_decoding_config` (mode Off/On/Adaptive, lenience).
- `litert_lm_engine_settings_set_selected_signatures` (1.1.0) — можно оставить только `prefill_*`, `decode`, `verify` (для MTP).
- `litert_lm_session_config_set_lora_path` — LoRA поддерживается (важно для Bonsai, если бы он был в `.litertlm`).
- Все функции, которые могут упасть, возвращают `LiteRtLmStatusCode`, результат — через `out_*`.

### 2.4. GPU backend

- На Android LiteRT-LM использует **OpenCL** (ML Drift) и требует `libOpenCL.so` + `libvndksupport.so`; в `AndroidManifest.xml` нужны:
  ```xml
  <uses-native-library android:name="libvndksupport.so" android:required="false"/>
  <uses-native-library android:name="libOpenCL.so" android:required="false"/>
  ```
- ML Drift — открытый (Apache-2.0) GPU-движок, поддерживает OpenCL, OpenGL ES, Metal, WebGPU; на Android приоритет OpenCL, fallback OpenGL ES.
- В LiteRT-LM есть собственный «Artisan» GPU backend для LLM (не только ML Drift); в 0.19.1 на нём включены Gemma 4 vision/audio.
- GPU даёт прежде всего **prefill**: S26 Ultra Gemma4-E4B — 1293 pp / 22.1 tg против 195 / 17.7 на CPU.
- **Ограничения:** на Adreno 750 подтверждена порча длинных промптов (issue #3012); на Adreno 840 — порча цифр после ~2048 токенов, лечится `activationDataType = FLOAT32` (issue #4001). На некоторых OEM (Samsung One UI) OpenCL discovery блокируется, fallback в OpenGL не реализован (issue #2292, v0.11; в 0.19 может быть иначе — проверять).
- **Adreno 630 (845):** OpenCL 2.0 и старый драйвер. Официально LiteRT GPU не тестируется на таком классе; ML Drift/OpenCL требует возможностей, которых у 630 может не быть (subgroups, fp16 storage). Даже если запустится через OpenGL ES fallback, для 3.66 GB LLM это не даст выигрыша. Вердикт: **не рассчитывать**.

### 2.5. NPU backend

- Поддерживаются Google Tensor, **Qualcomm QAIRT**, MediaTek NeuroPilot, Intel OpenVINO.
- Для Qualcomm нужны:
  - QAIRT SDK (например, **2.42.0.251225**; в документации LiteRT фигурирует именно эта версия);
  - dispatch-библиотеки, собранные под SoC: `@litert//litert/vendors/qualcomm/dispatch:dispatch_api_so`;
  - **per-SoC** `.litertlm`-файл (`..._sm8750.litertlm`, `..._sm8650.litertlm`, `..._sm8550.litertlm`);
  - на устройстве — `libQnnHtp*.so`, `libQnnSystem.so`, skels; для CLI — `ADSP_LIBRARY_PATH`, для app — `nativeLibraryDir`.
- NPU **не поддерживает speculative decoding** (C API: «the NPU backend decodes without speculation»); lenience на NPU игнорируется.
- Официальная таблица LiteRT NPU для Qualcomm (на момент проверки):

| Модель | SoC | Квант | Context | Размер |
|---|---|---|---:|---:|
| Gemma3-1B | SM8750 | 4-bit per-channel | 1280 | 658 MB |
| Gemma3-1B | SM8650 | 4-bit per-channel | 1280 | 658 MB |
| Gemma3-1B | SM8550 | 4-bit per-channel | 1280 | 657 MB |

- **Gemma 4 E4B в этой таблице нет.** Для SM8750 был файл E4B (см. 2.2), но в текущем tree его нет; E2B `sm8750` — есть. Для SM8650 (8 Gen 3) Gemma 4 **E4B** NPU-варианта нет; E2B NPU-bundle фигурирует в issue #4183 (там audio уходит на CPU и работает ~47 с) — то есть E2B на NPU возможен, E4B — нет.
- NPU-путь в LiteRT-LM требует **Early Access Program** (см. build-and-run: «check out this page … sign it up for an Early Access Program»).

### 2.6. Требования Android/NDK/ABI

| Требование | Значение |
|---|---|
| minSdk | 24 (AAR); проект — 28 |
| ABI | arm64-v8a (основной), x86_64 — только AAR/эмулятор |
| NDK для сборки из исходников | r28b или новее |
| 16 KB pages | AAR уже собран с `LOAD ... 0x4000` (проверено `llvm-readelf`) |
| libc++ | statically linked; `libc++_shared.so` не требуется (в `DT_NEEDED` нет) |
| GPU | `libOpenCL.so`, `libvndksupport.so` через `uses-native-library` |
| NPU | QAIRT libs + dispatch + per-SoC model; `ADSP_LIBRARY_PATH`/`nativeLibraryDir` |
| targetSdk | 36 (Android 16) — нормально |

### 2.7. Лицензия

- **Runtime — Apache-2.0.** Это позволяет линковать/распространять.
- **Веса Gemma 4 — Gemma Terms of Use**, не Apache. В карточке litert-community стоит `license: apache-2.0`, но это метаданные репозитория; юридически использование базовой модели регулируется Gemma Terms. Для продукта — отдельная проверка.
- **QAIRT/QNN — проприетарная лицензия Qualcomm.** Maven-артефакты опубликованы публично, но заголовок AAR: «Confidential and Proprietary — Qualcomm Technologies, Inc.». Перераспространение внутри APK требует чтения LICENSE.pdf/условий; для коммерческого продукта — legal review.

### 2.8. Линковка в Rust через C ABI

**Да, технически возможно, но не через Maven AAR.**

Факты:

- `liblitertlm_jni.so` из AAR экспортирует **только JNI-символы** (`Java_com_google_ai_edge_litertlm_LiteRtLmJni_*`); символов `litert_lm_*` в dynsym нет.
- В исходниках есть полноценный C API и цель `//c:litert-lm` — shared library, которая экспортирует только `litert_lm_*` (version script `litert_lm_c_api_linux.lds`, `-fvisibility=hidden`).
- C API версионирован: `LITERT_LM_C_API_VERSION_MAJOR/MINOR/PATCH`, `litert_lm_get_api_version`.
- CMake-сборка C API поддерживается (`c/CMakeLists.txt`, alias `LiteRTLM::C`), но основная сборка проекта — Bazel 7.6.1.

Практический рецепт для Rust:

1. Собрать `//c:litert-lm` под `android_arm64` с NDK r28b:
   ```bash
   bazel build --config=android_arm64 //c:litert-lm
   ```
   (или CMake-таргет `LiteRTLM::C`).
2. Забрать `liblitert-lm.so` + заголовки `c/engine.h`, `c/conversation.h`, `c/error_reporter.h`, `c/experimental.h`, `c/model_info.h`.
3. `bindgen` по этим заголовкам; линковать `-llitert-lm`.
4. C++ exceptions не должны пересекать FFI; C API уже отдаёт статусы, так что shim почти не нужен.
5. GPU-библиотеки LiteRT (ML Drift) должны лежать рядом; проверить `DT_NEEDED`/`dlopen` на устройстве.
6. Для NPU — `litert_lm_engine_settings_set_litert_dispatch_lib_dir`.

Альтернатива без Bazel: использовать **JNI** из Rust (`jni` crate), но тогда нужен JVM, Kotlin-классы и `NativeActivity`/`AndroidRuntime`; это тяжелее и хрупче, чем C ABI.

**Вывод:** если LiteRT-LM нужен как backend, закладывать 3–5 дней на сборку C API и обвязку. Если нужен только Gemma 4 — можно вместо этого взять GGUF + llama.cpp (Gemma 4 GGUF и MTP-драфтер уже опубликованы).

### 2.9. Gemma 4 E4B: 845 vs 8 Gen 3 vs 8 Elite

| Устройство | LiteRT CPU | LiteRT GPU | LiteRT NPU | Вердикт |
|---|---|---|---|---|
| **845 / 6 GB** | 🟠 технически возможно, RSS ~3.1–3.3 GB; на 6 GB — на грани OOM; оценка 30–45 pp / 2–3 tg | ❌/❓ OpenCL 2.0, старый драйвер; GL fallback не проверен | ❌ Hexagon v65, нет per-SoC bundle | **Не обещать.** Для 845 — E2B или Qwen2.5-0.5B |
| **8 Gen 3 / 16 GB** | 🟡 оценка 100–150 pp / 9–12 tg | 🟠 600–900 pp / 10–14 tg, но баг #3012 на промптах ~2k | ❌ E4B `sm8650` нет; только Gemma3-1B | CPU — безопасно; GPU — короткие промпты, тестировать |
| **8 Elite / 24 GB** | 🟡 оценка 140–180 pp / 13–16 tg | 🟡 900–1200 pp / 17–22 tg; Adreno 830, риск #4001-класса | ❓ E4B `sm8750` в текущем tree нет; E2B — да | GPU/MTP — основной путь; NPU — только после проверки файла |

**Почему 845 — память, а не только скорость.** Официальные замеры LiteRT: Raspberry Pi 5 16 GB, Gemma4-E4B CPU — 51 pp / 3.2 tg, RSS 3069 MB. Pi 5 (Cortex-A76 2.4 GHz, 16 GB) — пример «слабого, но 16-гигабайтного» класса. 845 (A75 2.8 GHz, 6 GB, без dotprod) будет медленнее, а главное — RSS те же ~3 GB при доступных ~3–3.5 GB на всё приложение. Это не «медленно», это «LMK может убить».

**Квантования Gemma 4 E4B:**

- Официальный `.litertlm` — **per-channel int4** (4-bit weights, per-channel scales); активации, вероятно, int8/fp16 в зависимости от варианта; точная схема E4B в карточке не расписана — проверять `litert-lm describe`.
- GGUF-варианты (для llama.cpp): Q4_0 **4.38 GB** (ggml-org) / 4.61 GB (unsloth), Q4_K_M 4.75 GB, Q3_K_S 3.68 GB, UD-IQ2_M 3.38 GB, Q2_K_XL 3.58 GB, Q8_0 7.66 GB, BF16 14.36 GB.
- Для 845 GGUF E4B **не влезает** ни в одном разумном кванте: 3.38–4.75 GB весов + KV + приложение > 6 GB.
- Для 8 Gen 3/Elite — Q4_0/Q4_K_M оптимальны по качеству/скорости; Q3_K_S только если экономить RAM.

### 2.10. Что реально поддерживается в LiteRT-LM (сводка)

| Возможность | CPU | GPU | NPU |
|---|---|---|---|
| Gemma 4 E4B | ✅ | ✅ (с оговорками) | ❓ per-SoC |
| Gemma 4 E2B | ✅ | ✅ | ✅ `sm8750` |
| Gemma 3n E4B | ✅ | ✅ | ❌ |
| Gemma 3 1B | ✅ | ✅ | ✅ `sm8650/sm8750` |
| Qwen2.5-0.5B/1.5B | ✅ | ✅ | ❌/❓ (нужен per-SoC `.litertlm`) |
| MTP / speculative | ✅ | ✅ | ❌ (no speculation) |
| Multimodal (vision/audio) | ✅ | ✅ | ✅ (зависит от bundle) |
| LoRA | ✅ | ❓ | ❓ |
| Tool calling | ✅ | ✅ | ✅ |
| 32k context | ✅ | ✅ | ограничено bundle (ekv1280 и т.п.) |

---

## 3. llama.cpp backends в 2026

**База:** upstream `ggml-org/llama.cpp`, commit **f7591a6** (2026-11-07); форк проекта — `external/llama-clover-src`, commit **4838ffb** (2026-10-28, PR #31306 + masked-block self-speculation). Лицензия MIT.

### 3.1. CPU (ARM NEON, dotprod, i8mm, KleidiAI)

| Фича | 845 | 8 Gen 3 | 8 Elite |
|---|---|---|---|
| NEON | ✅ | ✅ | ✅ |
| dotprod | ❌ | ✅ | ✅ |
| i8mm | ❌ | ✅ | ✅ |
| SVE/SVE2 | ❌ | ❌ (нет в Kryo) | ❌ (нет в Oryon) |
| SME | ❌ | ❌ | ❌ |
| KleidiAI | ⚠️ соберётся, но без dotprod/i8mm выигрыша нет | ✅ Q4_0/Q4_K/Q6_K/Q8_0 | ✅ |

Факты:

- `GGML_CPU_KLEIDIAI=ON` — официальная опция для Android `arm64-v8a`; runtime dispatch сам выбирает ядро по CPU features (dotprod/i8mm/SVE/SME). Сборка не требует глобального `-march=armv8.7a`; KleidiAI собирает свои ядра с нужными флагами.
- Поддерживаемые типы KleidiAI: **Q4_0, Q4_K, Q8_0, Q6_K**. Q2_0/Q1_0/TQ1_0/TQ2_0/MXFP4 — вне KleidiAI.
- На 845 (A75/A55 без dotprod/i8mm) KleidiAI не даст ускорения; останется обычный NEON-путь.
- Для Q2_0 (Ternary-Bonsai) на новых SoC работает штатный ARM-путь с dotprod; на 845 — медленно (факт: 2.3–2.6 tg).

**Практика для 845:** 4 потока (A75), `-t 4`, `taskset` на big-кластер, Q4_0/Q4_K_M; Q2_0 — только Bonsai; Q4_K на 845 деквантуется дорого, Q4_0 часто быстрее.

### 3.2. OpenCL (Adreno)

Официальный статус (docs/backend/OPENCL.md, ноябрь 2026):

| Параметр | Значение |
|---|---|
| ОС | Android, Windows 11 ARM64, Linux |
| Проверенные GPU | **Adreno 750 (8 Gen 3), 810 (7s Gen 3), 830 (8 Elite), 840 (8 Elite Gen 5), X1-85, X2-90** |
| A6x | «A6x GPUs with a recent driver and compiler are supported… A6x GPUs in phones are likely not supported due to the outdated driver and compiler» |
| Типы | Q1_0, Q4_0, Q4_1, Q5_0, Q5_1, Q8_0, Q4_K, Q5_K, Q6_K, MXFP4, IQ4_NL |
| Q2_0 | В docs-таблице ещё нет, но в коде есть `gemv/gemm_noshuffle_q2_0` (2026) |
| Adreno-ядра | `GGML_OPENCL_USE_ADRENO_KERNELS=ON` по умолчанию |
| Бинарная библиотека | `GGML_OPENCL_USE_ADRENO_BIN_KERNELS=ON` + `adreno-opencl-kernels.dll` (X2; вручную с Software Center) |
| Кэш программ | `GGML_OPENCL_KERNEL_CACHE_DIR` (по умолчанию включён) |

Qualcomm на IWOCL 2026 прямо называет OpenCL основным путём для Adreno: 50+ hand-tuned ядер, активная upstream-разработка; Vulkan для Adreno — отдельное направление, но не основной путь. Это подтверждает приоритет OpenCL над Vulkan на 750/830.

Что реально даёт OpenCL:

- **Prefill** — главный выигрыш: 1.5B Q4_0 на Adreno 750 — **252 pp / 24.1 tg**; 3B — **136 pp / 21.4 tg** (замеры, приведённые в отчёте 04; там же ссылка на PR #14841).
- 8 Elite, 1B F16 — ~199–275 pp / ~22 tg (замеры из отчёта 04 / PR #14987).
- Decode на GPU обычно **не быстрее CPU** или чуть быстрее; на маленьких моделях мешает launch overhead.
- MoE: есть `MUL_MAT_ID`, `TOP_K`, MoE-GEMM; Q4_0/Q4_K/MXFP4. Это делает Clover/gpt-oss потенциально интересными на 8 Gen 3/Elite, но для Clover нужны ещё SSM/block-diffusion ops.

**Adreno 630 / 845:**

- Формально backend собирается и видит GPU, но upstream считает A6x-телефоны неподдерживаемыми.
- Факт проекта: Qwen2.5-0.5B Q4_K — **3.05 pp / 2.23 tg** против 41.8 / 26.6 на CPU. Это не «чуть медленнее», а в 12–14 раз.
- Причина: generic-ядра, отсутствие Adreno-интринсиков, старый драйвер, launch/sync overhead, деквант без dotprod. Потолок по памяти (~30 GB/s) допускал бы ~1.5–2x, но реализация даёт 0.05x.
- **Вердикт: OpenCL на 845 не имеет смысла.** Оставить CPU.

**Известные баги OpenCL 2026:**

| Issue | Суть | Влияние |
|---|---|---|
| #31485 | Adreno 830: invalid work-group size в q6_K tiled GEMM | 8 Elite, Q6_K |
| #30182 | Оптимизации decode/prefill FA на Adreno (open) | перф, не корректность |
| #29081 | Adreno 730: неверные результаты MUL_MAT (закрыт) | старые 7xx |
| #30983 | Adreno X1-45: нет `cl_khr_subgroups`/`cl_qcom_subgroup_shuffle` | WoA, не телефон |
| #30412 | tiled q4_k GEMM для Adreno (закрыт) | +prefill |
| #30646 | trans-weight GEMM для узких q4_0 (a7x) | 750 |
| #30639 | f32_f16 FA на a7x | 750 |
| #27191 | KGSL throttling между submissions | MoE, tg |

### 3.3. Vulkan

- Backend существует, поддерживает Q4_K/Q5_K/Q6_K/IQ/TQ, MoE, FA.
- **845:** устройство видно (`Adreno 630, fp16:0, int dot:0`), но `device Vulkan0 does not support 16-bit storage`; патч fp32-only снимает проверку, но бинарник **SIGSEGV** в `ggml_vk_create_buffer`. Это закрытая дверь (факт проекта).
- **Adreno 750:** драйвер сегфолтится в `vkCreateComputePipelines` на matvec; нужен opt-in guard (PR #29165). FA: `cols_per_thread=0` (issue #30391).
- **Adreno 830:** vendor-драйвер делает SIGABRT на `-ngl>=1`; Turnip работает (issue #29786). SPIR-V от разных версий shaderc компилируется по-разному (issue #28635). Неверные результаты quantized `MUL_MAT` (issue #30654).
- **Adreno 840:** crash на TQ1_0 27B (issue #30649), wrong results (issue #30654).
- **8-bit pack/unpack** мисскомпилируется на нескольких Adreno (PR #30821).

**Вердикт:** Vulkan не использовать как основной путь на телефонах. На 845 — выключить. На 750/830 — только экспериментально, с Turnip/патчами и проверкой численной корректности.

### 3.4. QNN/Hexagon в llama.cpp

- Upstream `ggml-hexagon` — **не QNN**, а прямой HTP-backend через Hexagon SDK + FastRPC. Это важно: «QNN backend» в llama.cpp upstream нет; QNN EP — это ONNX Runtime, QNN delegate — LiteRT.
- `GGML_HEXAGON=ON` требует Hexagon SDK (в Snapdragon Docker — 6.7.0.0) и `HEXAGON_SDK_ROOT`/`HEXAGON_TOOLS_ROOT`.
- Собираются skels: **v73, v75, v79, v81, v85** (v65 нет — 845 отпадает).
- Hexagon-сессия — 32-битное адресное окно ~3.5 GB; для моделей больше — dynamic map/unmap или layer-split по виртуальным сессиям `HTP0:0,HTP0:1,...`.
- Поддержка типов: Q4_0, Q8_0, IQ4_NL, MXFP4, Q4_1, Q1_0, Q2_0, K-кванты (Q2_K/Q3_K/Q4_K/Q5_K/Q6_K), tiling 32×32, HVX/HMX.
- MoE: `MUL_MAT_ID`, `TOP_K`, `TOP_K_MASK`; но кастомные ops Clover (`ssm_bd_*`) — нет.
- Производительность (пример из docs, не воспроизведён): Llama-3.2-1B Q4_0 на v79 — 169 pp / 51.5 tg.
- Баги: v73 HMX `MUL_MAT` даёт inf при n>=5, FA/GDN ломаются (#29473); v73 garbled output на 8s Gen 3 (#31645); v75 vision encoder corruption (#31104); Qwen3-0.6B garbled (#16854, закрыт).

**Ключевой блокер — unsigned PD.** На Lenovo Y700 (8 Gen 3, Android 15) `ggml-hex` логирует:
```
ggml-hex: failed to enable unsigned PD for session 0 : error 0xffffffff
ggml-hex: failed to create device/session 0
```
То есть на retail-прошивке без root/инженерного режима backend может не запуститься. Это issue #19634. На OnePlus 13/Pad 2 надо проверять отдельно.

### 3.5. Сводная матрица backend × устройство

| Backend | 845 / Adreno 630 / v65 | 8 Gen 3 / Adreno 750 / v75 | 8 Elite / Adreno 830 / v79 |
|---|---|---|---|
| CPU NEON | ✅ работает; 41.8/26.6 на 0.5B | ✅ работает | ✅ работает |
| CPU + dotprod/i8mm | ❌ нет фич | ✅ | ✅ |
| KleidiAI | ⚠️ без выигрыша | ✅ Q4_0/Q4_K/Q6_K/Q8_0 | ✅ |
| OpenCL | ❌ 3.05/2.23 (0.5B), upstream не поддерживает A6x-телефоны | ✅ 252/24.1 (1.5B), 136/21.4 (3B); баги FA/q6_K | ✅ но q6_K workgroup bug #31485 |
| Vulkan | ❌ 16-bit storage → отказ; патч → SIGSEGV | 🟠 драйвер падает; нужен guard #29165 | 🟠 vendor abort #29786; Turnip; wrong results #30654 |
| Hexagon (ggml) | ❌ v65 нет | 🟠 v75 есть, но unsigned PD блокер | 🟠 v79 есть, unsigned PD блокер |
| QNN (QAIRT) | ❌ v65 | 🟠 v73+ формально; нужен per-SoC bundle | 🟠 v79; нужен bundle/QAIRT |

---

## 4. Qualcomm QNN / Hexagon / Genie

### 4.1. Что есть в 2026

| Компонент | Что это | Лицензия/доступ |
|---|---|---|
| **QAIRT** (Qualcomm AI Runtime, ex-QNN SDK) | Низкоуровневый runtime: QNN API, HTP/GPU/CPU backends, конвертеры, `qnn-net-run`, `genie-t2t-run` | Проприетарная; скачивание с Software Center, версия 2.42/2.48/2.49 |
| **QNN EP** | Execution Provider для ONNX Runtime | Проприетарная; `onnxruntime-android-qnn` |
| **QNN delegate for LiteRT** | Delegate TFLite/LiteRT → QNN HTP | Maven `com.qualcomm.qti:qnn-litert-delegate:2.48.0` |
| **Genie** | LLM runtime поверх QNN: bundle (context binaries + tokenizer + config), `genie-t2t-run` | В составе QAIRT |
| **GenieX** | Community-обёртка Qualcomm AI Hub: C SDK + CLI + Python + Kotlin/Java + OpenAI server; два runtime: `llama_cpp` (GGUF, CPU/GPU/Hexagon) и `qairt` (NPU, per-SoC bundle) | **BSD-3-Clause** (плюс Qualcomm Terms of Use) |
| **AI Hub** | Готовые модели/бандлы под конкретные SoC | Qualcomm AI Hub account |

Версии/артефакты (ноябрь 2026):

- QAIRT: в docs LiteRT — **2.42.0.251225**; в issue #21586 — 2.48.0/2.49.0.260730.
- QNN Maven: `com.qualcomm.qti:qnn-runtime:2.48.0` — AAR **68 393 877 B ≈ 68.4 MB**; распакованные `jni/arm64-v8a` — **≈197 MB**.
- GenieX Maven: `com.qualcomm.qti:geniex-android:0.10.0` — AAR **89 045 966 B ≈ 89 MB**; `jni/arm64-v8a` — **≈220 MB**, только arm64, minSdk **27**.
- GenieX C SDK: `libgeniex.so` (5 MB) экспортирует `geniex_*`; в AAR также `libllama.so`, `libggml-opencl.so`, `libggml-hexagon.so`, `libggml-htp-v73/v75/v79/v81/v85.so`, QNN V79/V81.

### 4.2. Форматы моделей и кванты

| Путь | Формат | Кванты | Где работает |
|---|---|---|---|
| **QNN/QAIRT** | DLC → context binary (`.bin`), model.so; Genie bundle (`.bin` + tokenizer + `genie_config.json`) | w4a16, w8a8, w8a16; per-channel; HTP | только NPU; per-SoC + per-QAIRT-version |
| **QNN delegate LiteRT** | `.tflite`/LiteRT + delegate | int8, int4 (зависит от модели) | NPU |
| **GenieX `qairt`** | AI Hub bundle | per-chipset, обычно w4a16 | NPU |
| **GenieX `llama_cpp`** | GGUF | Q4_0 (лучший для HTP), Q8_0, Q4_K, Q6_K, Q2_0, MXFP4 | CPU/OpenCL/Hexagon |
| **LiteRT-LM NPU** | `.litertlm` per-SoC | 4-bit per-channel | NPU |

Ограничения:

- QNN context binary **не переносится** между SoC и часто между версиями QAIRT; несовпадение `libQnnSystem.so` даёт ошибки (issue #2226).
- Genie требует HTP **v73+**; 845 (v65) — нет.
- GenieX `qairt` в AAR содержит skels только **V79/V81**, то есть на 8 Gen 3 (v75) этот путь в готовом AAR не заработает; там остаётся `llama_cpp` Hexagon v75.
- GenieX `llama_cpp` рекомендует **Q4_0**; Q4_K_M частично падает на CPU (не все тензоры на HTP).

### 4.3. GenieX — самый практичный «NPU + GGUF» путь

Почему важен:

- Один C SDK (`geniex.h`) → CLI, Python, Kotlin/Java, Docker, OpenAI-compatible server.
- Два runtime: `llama_cpp` (GGUF, CPU/OpenCL/Hexagon) и `qairt` (NPU, AI Hub bundles).
- Android AAR 89 MB zip (≈220 MB jni), minSdk 27, `libgeniex.so` экспортирует C-символы → **линкуется в Rust через C ABI**.
- BSD-3-Clause — проще с лицензией, чем QAIRT напрямую, но внутри всё равно QAIRT libs (см. 4.5).

Минусы:

- 89 MB zip / 220 MB jni; куча QNN/Prepare/skel-файлов, часть не нужна на конкретном устройстве.
- Свой форк llama.cpp; Clover/Bonsai/кастомные arch могут не поддерживаться.
- NPU-доступ всё равно упирается в unsigned PD/OEM.
- Версия 0.10.0, developer preview.

### 4.4. QNN EP (ONNX Runtime)

- `onnxruntime-android-qnn` + `qnn-runtime`; QNN EP.
- Поддержка HTP arch: 68, 69, 73, 75, 81; **v79 отсутствует** (PR #31638 не смержен на момент issue #21586).
- При ошибке инициализации QNN EP **молча** отдаёт граф CPU — это опаснее явного падения: приложение «работает», но не на NPU.
- Для LLM на телефоне ORT+QNN — не основной путь: нет GGUF, нужен ONNX + context binary; проще GenieX/QAIRT.

### 4.5. Подпись и unsigned PD — главный риск

Факты:

1. **llama.cpp / ggml-hexagon** на Lenovo Y700 (8 Gen 3, Android 15): `failed to enable unsigned PD for session 0 : error 0xffffffff`, сессия не создаётся (issue #19634).
2. **QNN delegate из обычного APK** на Samsung A36 (SM6475, Android 16): `createUnsignedPD unsigned PD or DSPRPC_GET_DSP_INFO not supported by HTP`, `Permission denied` на `/dev/fastrpc-cdsp`; при этом `qtld-net-run` из `adb shell` работает (issue qualcomm/ai-hub-apps#157).
3. **SM8750P / OnePlus/Xiaomi HyperOS**: `qnn-platform-validator` проходит, но из app — `QNN_DEVICE_ERROR_INVALID_CONFIG`; при этом Geekbench AI с собственным QNN-стеком на том же устройстве даёт HTP-ускорение (issue pytorch/executorch#21586). То есть доступ **возможен**, но зависит от QAIRT-версии/SoC-таблицы/политики OEM.
4. **Signed PD** требует подписанных skel/catalog; обычный разработчик не может подписать их вендорным ключом. QAIRT Community содержит `unsigned` skels; это и есть путь для разработки, но он не гарантирован на retail.

Вывод:

- **Нельзя обещать NPU на «любом Snapdragon».** Нужен per-device тест: `ls -l /dev/fastrpc-*`, `getprop ro.soc.model`, запуск минимального QNN/GenieX/ggml-htp теста из app UID.
- На OnePlus 13/Pad 2 шанс выше, чем на Samsung/HyperOS, но проверять.
- Fallback: CPU (всегда) → OpenCL (750/830) → NPU (если доступен).

### 4.6. Android 15/16

- **16 KB pages**: ARM-библиотеки QNN (`libQnnHtp.so`, `libQnnSystem.so`, `libQnnHtpPrepare.so`) выровнены на `0x4000` (проверено). Hexagon-skel — 32-битный ELF с `0x1000`; это DSP-библиотека, не грузится Android-линкером, но проверка Play/`zipalign` может её заметить — тестировать упаковку.
- **`uses-native-library`**: для OpenCL/FastRPC нужно объявить `libOpenCL.so`, `libvndksupport.so`, `libadsprpc.so`, `libcdsprpc.so` (GenieX так и делает).
- **W^X / dlopen из data**: не рассчитывать на скачивание `.so` в `filesDir` и `dlopen`; Android 14+ ограничивает исполнение записываемых файлов. Prebuilt-библиотеки — только в APK/nativeLibraryDir.
- **Foreground service**: длинный инференс — `dataSync`/`specialUse` + wake lock; иначе процесс/сессия могут быть убиты.
- **Thermal**: на 8 Gen 3/Elite длинный decode упирается в throttling; замеры делать после 2–3 минут прогрева и фиксировать `thermalservice`.

---

## 5. Спекулятивное декодирование

### 5.1. Что есть в llama.cpp (upstream, ноябрь 2026)

`--spec-type` (можно несколько через запятую):

| Тип | Что нужно | Комментарий |
|---|---|---|
| `draft-simple` | отдельная draft-модель (`-md`) | классика; draft должен быть сильно меньше target |
| `draft-mtp` | MTP/NextN-голова target-модели (`-md mtp-*.gguf` или `--mtp`) | MTP-контекст `LLAMA_CONTEXT_TYPE_MTP`, state embeddings |
| `draft-eagle3` | EAGLE-3 draft, обученный под target | 1-слойный transformer, читает hidden states target |
| `draft-dflash` | block-diffusion draft (DFlash) | выдаёт блок токенов за forward, GPU-friendly |
| `draft-dspark` | DSpark draft (DFlash + Markov head) | пока только Qwen3-backbone; Gemma4 — в планах |
| `draft-self` | ничего (self-speculation) | **в форке 4838ffb**, masked-block; нужен mask token |
| `ngram-cache` | ничего | статистика n-грамм |
| `ngram-simple` | ничего | поиск паттерна в истории |
| `ngram-map-k` / `k4v` | ничего | хэш-таблица n-грамм |
| `ngram-mod` | ничего (~16 MB) | rolling hash, общий пул; лучший «бесплатный» вариант |

Ключевые флаги: `--spec-draft-n-max` (по умолчанию 3), `--spec-draft-n-min`, `--spec-draft-p-min`, `--spec-draft-ngl`, `--spec-draft-device`, `--spec-draft-threads`, `--spec-draft-type-k/v`, `--spec-draft-override-tensor`.

### 5.2. MTP (`LLAMA_CONTEXT_TYPE_MTP`) — детали

- В `llama.h` есть `enum llama_context_type { DEFAULT, MTP }`, `llama_model_params.load_mtp`, `llama_batch_ext_set_embd_state` (state embedding от target-модели).
- MTP-голова — это не отдельная полноценная модель, а несколько слоёв, обученных предсказывать следующие токены по hidden states target. Поэтому файл маленький: **Gemma 4 E4B MTP Q4_0 — 57 MB**, Q8_0 — 94 MB, BF16 — 164 MB.
- Кто поддерживает MTP в текущем дереве (по `supports_mtp_export`): DeepSeek V4/V4.1, GLM, Qwen3-Next/3.5 (MoE), Hunyuan, Nemotron, Step3, BailingMoE3, Dots3, Mimo, Mellum, **Gemma 4 Assistant** (gemma.py: `Gemma4AssistantModel`).
- Для Gemma 4 E4B опубликованы `mtp-gemma-4-E4B-it-Q4_0.gguf` (57 MB), Q8_0 (94 MB), BF16 (164 MB) — можно использовать уже сегодня.
- Команда (пример):
  ```bash
  llama-cli -m gemma-4-E4B-it-Q4_0.gguf \
            -md mtp-gemma-4-E4B-it-Q4_0.gguf \
            --spec-type draft-mtp \
            -ngl 99 -c 4096
  ```
- Ограничения на телефоне:
  - MTP-контекст требует target hidden states; на Hexagon это может быть не поддержано (проверять).
  - Verify-батч увеличивает compute; на memory-bound 845 выигрыша не будет.
  - `max_output_tokens` в LiteRT-MTP считает draft-токены (issue #2816); в llama.cpp это не так, но свои лимиты надо проверять.
  - Для MTP в приложении нужен **common/speculative** слой; в `libllama` его нет (см. 5.5).

### 5.3. Clover: masked-block self-speculation (`draft-self`)

- Форк 4838ffb (PR #31306) добавляет `COMMON_SPECULATIVE_TYPE_DRAFT_SELF` — self-speculative decoding с masked-block драфтами.
- Требования (из кода): causal target context с memory, **mask token** в модели, положительная draft capacity; при ошибке — warning и продолжение без драфта.
- Для Clover 35B-A3B это единственный «бесплатный» спекулятивный путь без отдельной draft-модели.
- На 845 Clover storage-bound (0.6 tg/s), draft-self может дать лишь небольшой эффект; на 8 Elite с 24 GB — потенциально интересно, но замеров нет.
- В upstream/mainline этого типа нет; в Clover GGUF должны быть block-diffusion тензоры (`ssm_bd_history`, `ssm_bd_logit_scale` и т.д.).

### 5.4. LiteRT-LM MTP

- Драфтер **внутри `.litertlm`**; отдельный файл не нужен.
- Включается:
  ```kotlin
  @OptIn(ExperimentalApi::class)
  ExperimentalFlags.enableSpeculativeDecoding = true
  ```
  или C API `litert_lm_speculative_decoding_config_set_mode(ON/ADAPTIVE)`.
- Работает на **CPU и GPU**; **NPU — без speculation**.
- `lenience=0` — lossless; `>0` — быстрее, но качество может отличаться.
- Официальные замеры S26 Ultra (Gemma4-E4B):
  - CPU baseline 17.0 → summarize 27.5, code 26.2, rewrite 29.5, free form 21.1 tg/s;
  - GPU baseline 21.9 → summarize 46.0, code 49.4, rewrite 47.5, free form 36.7 tg/s.
- Баг #2816: draft-токены считаются в `maxOutputTokens`, GPU генерирует ~47% запрошенного. Workaround: ставить лимит с запасом или выключить MTP до фикса.

### 5.5. Что реально сделать на телефоне

| Устройство | MTP | draft-self | n-gram | draft model | EAGLE3/DFlash/DSpark |
|---|---|---|---|---|---|
| 845 / 6 GB | ❌ память/compute; E4B не влезает | 🟠 Clover, но storage-bound | ✅ лучший вариант | 🟠 0.5B draft к 0.5B бессмысленно; к 3B — можно, но память | ❌ |
| 8 Gen 3 / 16 GB | ✅ Gemma4 E4B MTP (57 MB) — главный кандидат | 🟠 Clover, 16 GB на грани | ✅ | 🟡 Qwen2.5-0.5B как draft к 1.5–3B | 🟠 EAGLE3 для Qwen3-1.7B; DFlash/DSpark — нет |
| 8 Elite / 24 GB | ✅ Gemma4 E4B MTP; Qwen3-Next/3.5 MTP | ✅ Clover + draft-self (потенциал) | ✅ | ✅ | 🟠 DFlash/DSpark на GPU, но draft 4B |

**Интеграционный нюанс.** В llama.cpp спекулятивный цикл живёт в `common/speculative.cpp`, а не в `libllama`. В проекте уже линкуется `llama-common` (отчёт 10), значит, нужно добавить C-shim над `common_speculative` (init/begin/draft/accept/stats), а не ждать `llama_decode`--only API. Для MTP также нужен `llama_batch_ext_set_embd_state` и контекст `LLAMA_CONTEXT_TYPE_MTP`.

**По нашим четырём моделям:**

| Модель | MTP | Что реально доступно |
|---|---|---|
| Gemma 4 E4B | ✅ | `mtp-gemma-4-E4B-it-Q4_0.gguf` 57 MB (или Q8_0/BF16); `--spec-type draft-mtp` |
| Clover 35B-A3B | ❌ (`supports_mtp_export=False`) | только `draft-self` в форке 4838ffb; в upstream — n-gram |
| Ternary-Bonsai 2-4B Q2_0 | ❌ (у Q2_0 нет головы) | PQ2_0-MTP — отдельный формат/PrismML-форк; DFlash2-BF16 — отдельный драфтер, 1.57 GB |
| Qwen2.5-0.5B | ❌ (MTP только у Qwen3-Next/3.5) | draft-модель (но 0.5B в роли draft к 0.5B бессмысленна) или n-gram |

---

## 6. Практическая матрица

> **Внимание:** только строки, помеченные «факт», измерены на устройстве. Остальное — оценки на основе соседних SoC/литературы; их нельзя публиковать как результаты.

### 6.1. MI 8 / SD845 / 6 GB

| Модель | CPU | OpenCL | Vulkan | QNN/Hexagon | LiteRT GPU | LiteRT NPU |
|---|---|---|---|---|---|---|
| **Gemma 4 E4B (LiteRT)** | 🟠 30–45 pp / 2–3 tg (оценка по Pi5 51/3.2); RSS 3.1–3.3 GB → **OOM риск** | ❌ 3.05/2.23 на 0.5B (факт); E4B не влезет | ❌ 16-bit storage → отказ; патч → SIGSEGV (факт) | ❌ v65 | ❌/❓ OpenCL 2.0; GL fallback не проверен | ❌ нет bundle |
| **Gemma 4 E4B (GGUF Q4_0)** | 💀 4.38–4.6 GB весов + KV > 6 GB | ❌ | ❌ | ❌ | — | — |
| **Clover 35B-A3B (GGUF)** | 🟠 **2.4 pp / 0.6 tg (факт)**; storage-bound, 11.69 GB | ❌ | ❌ | ❌ | — | — |
| **Ternary-Bonsai 2-4B Q2_0 + LoRA** | ✅ **3.1–3.7 pp / 2.3–2.6 tg (факт)**; LoRA работает | ❌/❓ Q2_0-ядра есть, но GDN/SSM и драйвер 630 | ❌ | ❌ | — | — |
| **Qwen2.5-0.5B Q4_K_M** | ✅ **41.8 pp / 26.6 tg (факт)** | 🟠 **3.05 / 2.23 (факт)** — хуже CPU | ❌ | ❌ | 🟠 официальный LiteRT — `.task`/`.tflite` dynamic int8 521 MB; для LiteRT-LM нужен community `.litertlm` или конвертация | ❌ |

**Рекомендация для 845:** Qwen2.5-0.5B CPU (UX), Bonsai Q2_0 CPU (качество/размер), Clover — только демо. Gemma 4 E4B — не обещать; если нужен Gemma — E2B и только после теста памяти.

### 6.2. OnePlus Pad 2 / 8 Gen 3 / 16 GB

| Модель | CPU | OpenCL | Vulkan | Hexagon (ggml) | LiteRT GPU | LiteRT NPU |
|---|---|---|---|---|---|---|
| **Gemma 4 E4B (LiteRT)** | 🟡 100–150 pp / 9–12 tg (оценка) | — | — | — | 🟠 600–900 / 10–14; **баг #3012** на ~2k | ❌ E4B `sm8650` нет; E2B NPU bundle есть (#4183); Gemma3-1B — ✅ |
| **Gemma 4 E4B (GGUF Q4_0)** | 🟡 80–120 / 8–11 | 🟡 500–800 / 10–14 | 🟠 драйвер | ❓ | — | — |
| **Clover 35B-A3B** | 🟠 10–20 / 3–6; 11.7 GB + система — на грани | 🟠 prefill; decode — нет | ❌ | ❌ кастомные ops | — | — |
| **Bonsai Q2_0 + LoRA** | 🟡 8–15 / 6–10 | 🟡 prefill | ❌ | ⚠️ Q2_0 tile есть, GDN/SSM — нет | — | — |
| **Qwen2.5-0.5B** | 🟡 70–100 / 40–60 | 🟡 100–200 / 30–45 | 🟠 | 🟡 50–100 | 🟠 official `.task`/`.tflite` int8; `.litertlm` — community/конвертация | ❌/❓ |

**Рекомендация:** LiteRT CPU для Gemma 4 E4B (безопасно), GGUF CPU как второй путь; OpenCL — для prefill/коротких промптов, с проверкой #3012. NPU — только Gemma3-1B как демо.

### 6.3. OnePlus 13 / 8 Elite / 24 GB

| Модель | CPU | OpenCL | Vulkan | Hexagon (ggml) | LiteRT GPU | LiteRT NPU |
|---|---|---|---|---|---|---|
| **Gemma 4 E4B (LiteRT)** | 🟡 140–180 / 13–16 | — | — | — | 🟡 900–1200 / 17–22; Adreno 830 | ❓ E4B `sm8750` в tree нет; E2B — ✅ |
| **Gemma 4 E4B (GGUF Q4_0)** | 🟡 120–160 / 12–15 | 🟡 800–1100 / 15–20 | 🟠 | ❓ | — | — |
| **Clover 35B-A3B** | ✅ 15–25 / 8–14 (оценка; 24 GB хватает) | 🟠 prefill | ❌ | ❌ | — | — |
| **Bonsai Q2_0 + LoRA** | ✅ 12–20 / 10–16 | 🟡 | ❌ | ⚠️ | — | — |
| **Qwen2.5-0.5B** | ✅ 100–130 / 60–90 | 🟡 150–250 / 40–60 | 🟠 | 🟡 | 🟡 | ❌/❓ |

**Рекомендация:** Clover CPU — главный кейс 24 GB; Gemma 4 E4B GPU + MTP — второй; NPU — через GenieX/QAIRT после проверки доступа.

### 6.4. Сводка «что запускать сегодня»

| Сценарий | 845 | Pad 2 | OnePlus 13 |
|---|---|---|---|
| Быстрый чат | Qwen2.5-0.5B CPU | Qwen2.5-0.5B CPU/OpenCL | Qwen2.5-0.5B CPU/OpenCL |
| Gemma 4 | E2B (только после теста RAM) | E4B LiteRT CPU | E4B LiteRT GPU + MTP |
| Clover | демо (0.6 tg/s) | только 2k ctx, риск OOM | ✅ основной кейс |
| Bonsai + LoRA | ✅ 2.3–2.6 tg/s | 🟡 | ✅ |
| NPU | ❌ | Gemma3-1B демо | GenieX/QAIRT, если доступен |

---

## 7. План внедрения

### 7.1. Зависимости и файлы

| Компонент | Артефакт | Куда |
|---|---|---|
| llama.cpp | `external/llama.cpp` (f7591a6) / `llama-clover-src` (4838ffb) | уже есть; CMake |
| KleidiAI | submodule/`GGML_CPU_KLEIDIAI=ON` | build flag |
| OpenCL headers/ICD | Khronos + NDK `libOpenCL.so` | CMake |
| Hexagon SDK | 6.7.0.0 (Docker `ghcr.io/snapdragon-toolchain/arm64-android:v0.8`) | `GGML_HEXAGON=ON` |
| LiteRT-LM | AAR `litertlm-android:0.19.1` **или** `liblitert-lm.so` из `//c:litert-lm` | `jniLibs/arm64-v8a` или Rust sys-crate |
| GenieX | AAR `geniex-android:0.10.0` **или** `libgeniex.so` + `geniex.h` | отдельный flavor/процесс |
| QAIRT (опционально) | `libQnnHtp*.so`, `libQnnSystem.so`, skels | `jniLibs`/`assets` |
| Модели | `.litertlm`, GGUF, MTP-драфтеры | скачивание, не в APK |

**Нельзя** линковать LiteRT-LM, GenieX и свой llama.cpp в один процесс без разбора конфликтов: GenieX везёт свои `libllama.so`/`libggml-*.so`, LiteRT — свой JNI. Варианты: (а) отдельные APK/flavor; (б) отдельный процесс для NPU; (в) один backend на продукт.

### 7.2. Feature flags (Rust)

```toml
[features]
backend-llama-cpu      = []
backend-llama-opencl   = []
backend-llama-hexagon  = []
backend-litertlm       = []   # C ABI, liblitert-lm.so
backend-geniex         = []   # C ABI, libgeniex.so
spec-ngram             = []
spec-mtp               = []
spec-self              = []
model-gemma4           = []
```

Логика выбора на устройстве:

```
soc = getprop ro.soc.model
if 845:            CPU only; models: Qwen2.5-0.5B, Bonsai Q2_0, E2B
if SM8650:         CPU + optional OpenCL; LiteRT CPU/GPU; no E4B NPU
if SM8750/SM8850:  CPU + OpenCL + optional Hexagon/QAIRT
```

### 7.3. Build matrix

| Вариант | Для кого | Флаги | Ожидаемый размер |
|---|---|---|---|
| `baseline845` | MI 8 | `-march=armv8-a`, `GGML_OPENCL=OFF`, `GGML_VULKAN=OFF`, `GGML_HEXAGON=OFF`, `GGML_CPU_KLEIDIAI=OFF`, `-t 4` | ~20–30 MB |
| `modern` | Pad 2 / OnePlus 13 | `-march=armv8.6-a+dotprod+i8mm`, `GGML_CPU_KLEIDIAI=ON`, `GGML_OPENCL=ON`, `GGML_HEXAGON=ON`, `GGML_VULKAN=OFF` | ~35–50 MB |
| `litert` | Gemma 4 | `modern` + LiteRT-LM AAR/C API | ~55–70 MB |
| `full` | R&D | `modern` + GenieX AAR | ~230–260 MB без trim; ~60–80 MB после trim v79 |

**Про «v86/v90».** Это не версии Android и не поколения Snapdragon, а уровни/наборы ARM-инструкций. Практический смысл для нас:

- **845** — ARMv8.2-A без dotprod/i8mm; единственный безопасный baseline — `armv8-a` (или `armv8.2-a` без dotprod).
- **8 Gen 3** — ARMv9.2 (X4/A720/A520), но из «полезного» нам нужны прежде всего **dotprod + i8mm + fp16**; `-march=armv8.6-a+dotprod+i8mm` — достаточный и переносимый вариант.
- **8 Elite** — Oryon (ARMv9.2-класс), тоже dotprod/i8mm; отдельная сборка «v90» **не нужна**, пока нет замеров, что она даёт выигрыш. Опасность `-march=armv9-a`/`v90` в том, что бинарник может не запуститься на 8 Gen 3/старых ядрах, а выигрыш от новых инструкций в llama.cpp для наших моделей не доказан.
- Один `modern`-бинарник с `armv8.6-a+dotprod+i8mm` + KleidiAI покрывает Pad 2 и OnePlus 13; 845 — отдельный baseline. Проверять фактический набор по `/proc/cpuinfo` (`Features: fp asimd ... dotprod i8mm`).

Ключевые CMake-флаги (проверены в проекте):

```
-DGGML_NATIVE=OFF
-DGGML_OPENMP=OFF
-DGGML_LLAMAFILE=OFF
-DGGML_BACKEND_DL=OFF          # для baseline; ON — для экспериментов
-DGGML_CPU_KLEIDIAI=ON|OFF
-DGGML_CPU_ALL_VARIANTS=OFF    # не раздувать; KleidiAI сам диспатчит
-DLLAMA_BUILD_EXAMPLES=OFF
-DLLAMA_BUILD_TOOLS=OFF
-DLLAMA_BUILD_SERVER=OFF
-DLLAMA_CURL=OFF
-DANDROID_STL=c++_static
-DANDROID_PLATFORM=28
```

Линкер: `-Wl,-z,max-page-size=16384`.

### 7.4. 16 KB, libc++_shared, ABI

| Вопрос | Ответ |
|---|---|
| 16 KB pages | NDK r28 по умолчанию; проверить `llvm-readelf -l` → `LOAD ... 0x4000`; `zipalign -c -P 16` |
| LiteRT AAR | уже `0x4000` (проверено) |
| GenieX AAR | уже `0x4000` (проверено) |
| QNN ARM | `libQnnHtp.so`, `libQnnSystem.so`, `libQnnHtpPrepare.so` — `0x4000` (проверено) |
| QNN skels | 32-битный Hexagon ELF, `0x1000`; это нормально, но проверить упаковку/Play |
| libc++_shared | не нужен ни LiteRT, ни GenieX, ни QNN (statically linked; `DT_NEEDED` только libc/libm/libdl/liblog) |
| ABI | только `arm64-v8a`; `armeabi-v7a` не нужен (845 — 64-bit) |
| minSdk | проект 28; LiteRT 24; GenieX 27 |
| targetSdk | 36 (Android 16) |

**Упаковка:**

```kotlin
android {
  defaultConfig { ndk { abiFilters += "arm64-v8a" } }
  packaging { jniLibs { useLegacyPackaging = false } }
}
```

`android:extractNativeLibs="false"` в манифесте; иначе `.so` распакуются и 16 KB-выравнивание APK теряет смысл.

### 7.5. Размер APK (оценка)

| Слой | Размер |
|---|---:|
| Текущий Rust+Slint APK (факт, отчёт 11) | 8.5 MB |
| + llama.cpp CPU/OpenCL/Hexagon (stripped) | +10–18 MB |
| + LiteRT-LM AAR (jni 21.6 MB, uncompressed) | +22 MB |
| + GenieX AAR: все jni **220 MB** (AAR 89 MB — это сжатый zip) | +220 MB; **trimmed v79 ≈ 45–50 MB** |
| + QNN: все jni **197 MB** (AAR 68 MB) | trimmed v79 без `Prepare` ≈ 26 MB (если используются только precompiled context binaries) |
| Модели | не в APK |

Важно: при `extractNativeLibs=false`/`useLegacyPackaging=false` `.so` лежат в APK **без сжатия** и page-aligned, поэтому считать надо по сумме jni, а не по размеру AAR. GenieX/QNN «как есть» — это +200 MB к APK; для продукта обязателен trim (оставить только нужный HTP-арх, убрать `Prepare`, profiling, `PlatformValidator`, V81 и т.д.).

Реалистичные варианты: «тонкий» 20–30 MB; «Gemma» 55–70 MB; «GenieX trimmed» 60–80 MB; «полный R&D без trim» 230–260 MB. Для Play — App Bundle + dynamic feature по SoC/бэкенду, но native libs нельзя скачивать и `dlopen` из data; лучше отдельные APK для R&D.

### 7.6. Rust FFI: три варианта

| Вариант | Плюсы | Минусы | Решение |
|---|---|---|---|
| **llama.cpp C ABI** (свой sys-crate) | уже есть; полный контроль; MIT | нужен shim для speculative/common | основной |
| **LiteRT-LM C ABI** | Gemma 4, MTP, multimodal, NPU | AAR не экспортирует C; сборка Bazel/NDK; ~3–5 дней | optional feature |
| **GenieX C ABI** | GGUF + CPU/GPU/NPU «из коробки»; BSD-3 | 89 MB zip / 220 MB jni; свой llama.cpp; preview | optional feature |
| **JNI (LiteRT AAR)** | не надо Bazel | JVM, Kotlin-классы, хрупко | не рекомендовать |

Для LiteRT C ABI:

```rust
// bindgen по c/engine.h, c/conversation.h, c/error_reporter.h
#[link(name = "litert-lm")]
extern "C" {
    fn litert_lm_engine_settings_create(...) -> LiteRtLmStatusCode;
    fn litert_lm_engine_create(...) -> LiteRtLmStatusCode;
    // ...
}
```

Обязательно: `catch_unwind` на границе, не давать C++ exceptions/ Rust panic пересекать FFI; проверять `litert_lm_get_api_version`.

### 7.7. Порядок работ

| Шаг | Что | Время | Критерий готовности |
|---|---|---|---|
| 0 | Замер памяти/перфа на трёх устройствах текущим APK | 1 день | таблица RSS/ttft/tg |
| 1 | LiteRT-LM C API build + smoke (845 CPU, Pad 2 GPU) | 3–5 дней | `litert_lm_main --benchmark` на устройстве |
| 2 | Gemma 4 E4B GGUF + MTP в llama.cpp shim | 3–5 дней | acceptance rate, tg/s на Pad 2/13 |
| 3 | n-gram spec для 845 | 1 день | +10–30% на повторяющемся тексте |
| 4 | GenieX/QAIRT smoke на OnePlus 13 | 2–3 дня | HTP-сессия из app UID |
| 5 | Упаковка/16 KB/размеры | 2 дня | `zipalign -c -P 16`, install |
| 6 | Решение по NPU | после 1–4 | включать только если есть стабильный app-доступ |

**Минимальный аудит на устройстве (до любых обещаний):**

```powershell
adb shell getprop ro.soc.model
adb shell getprop ro.board.platform
adb shell getprop ro.build.version.sdk
adb shell "cat /proc/meminfo | grep -E 'MemTotal|MemAvailable|SwapFree'"
adb shell "ls -l /dev/fastrpc-* 2>/dev/null"
adb shell "dumpsys thermalservice | grep -i 'Thermal Status'"
adb shell "ls /vendor/lib/rfsa/adsp 2>/dev/null | head"
```

Проверка 16 KB:

```powershell
& "$env:ANDROID_HOME\build-tools\36.0.0\zipalign.exe" -c -P 16 -v 4 app\build\outputs\apk\release\app-release.apk
& "$env:ANDROID_HOME\ndk\28.2.13676358\toolchains\llvm\prebuilt\windows-x86_64\bin\llvm-readelf.exe" -l app\src\main\jniLibs\arm64-v8a\libsnapcore.so | Select-String LOAD
```

Проверка, что NPU реально используется (а не молчаливый CPU-fallback): логи `QnnDsp`/`ggml-hex`/`GenieX` + профиль delegated nodes; для LiteRT — `--enable_profiling`/per-op профиль.

---

## 8. Риски и что нельзя обещать без проверки на устройстве

| Риск | Где | Вероятность | Последствие | Митигация |
|---|---|---|---|---|
| Gemma 4 E4B не влезает в 6 GB | 845 | высокая | OOM/LMK, краш | E2B; лимит ctx; мониторинг `meminfo` |
| OpenCL на 845 «работает», но медленный | 845 | факт | UX хуже CPU | не включать |
| Vulkan на 845 не работает | 845 | факт | краш | выключить |
| LiteRT GPU портит длинный текст на 750 | Pad 2 | подтверждено (#3012) | неверные ответы | CPU; `FLOAT32`; короткие промпты |
| MTP считает draft-токены в лимит | LiteRT GPU | подтверждено (#2816) | ответ ~47% длины | лимит x2; выключить MTP |
| NPU недоступен из app UID | Pad 2 / 13 | средняя | NPU не работает | fallback CPU/GPU; тест до обещаний |
| QNN context binary не совпал с QAIRT/SoC | 13 | средняя | ошибка/мусор | фиксировать версии; per-SoC bundle |
| Hexagon unsigned PD запрещён | retail | средняя | backend не стартует | signed PD/вендор; fallback |
| Adreno 830: q6_K workgroup bug | 13 | подтверждено (#31485) | неверный/битый вывод | Q4_0; ждать фикс |
| Adreno 830 Vulkan: неверные quantized результаты | 13 | подтверждено (#30654) | мусор | Vulkan off |
| 16 KB проверка ломается на QNN skel | все | низкая/средняя | Play reject/install | тест `zipalign`; skels в assets |
| Clover не влезает в 16 GB | Pad 2 | высокая | OOM | 2k ctx; только 24 GB |
| Thermal throttling | 8 Gen 3/13 | высокая | −30–50% tg | профили; `thermalservice` |
| Лицензии Gemma/QAIRT | продукт | средняя | юридический блок | legal review; GenieX BSD-3 не отменяет QAIRT |

**Чего нельзя обещать:**

1. «Gemma 4 E4B работает на MI 8» — нет, память.
2. «LiteRT GPU ускорит 845» — нет.
3. «NPU есть на всех Snapdragon» — нет, v65/v75 и OEM-политика.
4. «MTP даёт 3x на телефоне» — нет; 1.3–2x в лучшем случае, зависит от задачи.
5. «Clover 35B идёт на 16 GB с большим контекстом» — нет, на грани.
6. «QNN можно просто добавить в APK» — нет, per-SoC bundle + unsigned PD.
7. «16 KB готово» — только после `readelf`/`zipalign` на конкретных `.so`.
8. «OpenCL на 845 быстрее CPU» — ровно наоборот (факт).

---

## 9. Краткое резюме, blocker-лист, следующие шаги

### Резюме

- **LiteRT-LM** — зрелый (v0.19.1), Apache-2.0, лучший путь для Gemma 4: MTP внутри `.litertlm`, GPU/prefill, NPU per-SoC. Но NPU — Early Access и per-SoC, а Android AAR — JNI-only.
- **llama.cpp** — лучший путь для GGUF (Clover, Bonsai, Qwen), MIT, CPU/OpenCL/Hexagon, MTP/EAGLE3/DFlash/DSpark/ngram. На 845 — только CPU; на 8 Gen 3/Elite — CPU + OpenCL, Hexagon — после проверки unsigned PD.
- **GenieX** — самый практичный «NPU + GGUF» turnkey C SDK (BSD-3, 89 MB zip / 220 MB jni), но preview, свой llama.cpp и те же QAIRT-ограничения.
- **845** — Qwen2.5-0.5B CPU (26.6 tg/s), Bonsai Q2_0 CPU (2.3–2.6 tg/s), Clover только демо. Gemma 4 E4B — нет; E2B — с оговорками.
- **Pad 2** — Gemma 4 E4B LiteRT CPU (безопасно) или GGUF CPU; GPU — короткие промпты, баг #3012; Clover — на грани 16 GB.
- **OnePlus 13** — Clover CPU (24 GB) и Gemma 4 E4B GPU + MTP; NPU — только после теста app-доступа.

### Blocker-лист

1. **845 + Gemma 4 E4B:** 3.66 GB (LiteRT) / 4.4 GB (GGUF Q4_0) при 6 GB RAM → OOM. Нужен E2B или отказ.
2. **845 GPU/NPU:** OpenCL 2.0 + A6x not supported; Vulkan 16-bit storage/SIGSEGV; Hexagon v65 → нет пути.
3. **LiteRT-LM Android AAR не экспортирует C API:** для Rust нужна сборка `//c:litert-lm` (Bazel/NDK) или JNI.
4. **NPU app access:** unsigned PD/FastRPC может быть запрещён OEM; signed PD требует вендорной подписи.
5. **Gemma 4 E4B NPU на SM8650:** официального bundle нет; на SM8750 E4B-файл в текущем tree отсутствует.
6. **16 GB + Clover:** 11.69 GB + система + KV — на грани; только 24 GB надёжно.
7. **LiteRT GPU на Adreno 750:** подтверждённая порча длинных промптов (#3012); на 830/840 — риск порчи цифр (#4001).
8. **MTP в LiteRT:** draft-токены считаются в лимит (#2816); в llama.cpp спекулятивный цикл не в `libllama`.
9. **QNN лицензия/редистрибуция:** «Confidential and Proprietary»; нужен legal review, особенно для Play.
10. **16 KB + QNN skels:** ARM-части aligned, Hexagon-skel 4 KB; проверить упаковку и Play-чек.

### Следующие шаги (приоритет)

1. **Сегодня:** снять на трёх устройствах `ro.soc.model`, `ro.board.platform`, `MemAvailable`, `thermalservice`, `ls /dev/fastrpc-*`; зафиксировать версии QAIRT/Android.
2. **1–2 дня:** прогнать текущий APK: Qwen2.5-0.5B CPU, Bonsai Q2_0, Clover (2k ctx) — получить базовую таблицу tg/ttft/RSS.
3. **3–5 дней:** собрать LiteRT-LM C API под arm64; smoke Gemma 4 E4B CPU на 845 (только память) и GPU на Pad 2 (короткий/длинный промпт, проверить #3012).
4. **3–5 дней:** добавить Gemma 4 E4B GGUF + `draft-mtp` в llama.cpp shim; замерить acceptance/tg на Pad 2 и OnePlus 13.
5. **2–3 дня:** GenieX/QAIRT smoke на OnePlus 13 из app UID; если HTP-сессия не открывается — зафиксировать и не включать NPU.
6. **После замеров:** решить, нужен ли LiteRT-LM как продуктовый backend или достаточно GGUF+llama.cpp; NPU — только как opt-in с fallback.

---

## 10. Источники

### LiteRT-LM

- Репозиторий: https://github.com/google-ai-edge/LiteRT-LM
- Overview: https://ai.google.dev/edge/litert-lm/overview
- Android/Kotlin: https://ai.google.dev/edge/litert-lm/android
- NPU: https://ai.google.dev/edge/litert/next/litert_lm_npu
- Build from source: https://github.com/google-ai-edge/LiteRT-LM/blob/main/docs/getting-started/build-and-run.md
- C API: https://github.com/google-ai-edge/LiteRT-LM/blob/main/c/engine.h
- C BUILD target: https://github.com/google-ai-edge/LiteRT-LM/blob/main/c/BUILD
- Conversation API: https://github.com/google-ai-edge/LiteRT-LM/blob/main/docs/api/cpp/conversation.md
- File builder: https://developers.google.com/edge/litert-lm/file_builder
- Gemma 4 E4B: https://huggingface.co/litert-community/gemma-4-E4B-it-litert-lm
- Maven: https://maven.google.com/web/index.html#com.google.ai.edge.litertlm:litertlm-android
- ML Drift: https://developers.googleblog.com/ml-drift-next-gen-gpu-aiml-inference-at-the-edge/
- Issues: [#3012](https://github.com/google-ai-edge/LiteRT-LM/issues/3012), [#2292](https://github.com/google-ai-edge/LiteRT-LM/issues/2292), [#2816](https://github.com/google-ai-edge/LiteRT-LM/issues/2816), [#4001](https://github.com/google-ai-edge/LiteRT-LM/issues/4001), [#2226](https://github.com/google-ai-edge/LiteRT-LM/issues/2226), [#4183](https://github.com/google-ai-edge/LiteRT-LM/issues/4183), [#4185](https://github.com/google-ai-edge/LiteRT-LM/issues/4185), [#3867](https://github.com/google-ai-edge/LiteRT-LM/issues/3867)

### llama.cpp

- Репозиторий (f7591a6, 2026-11-07): https://github.com/ggml-org/llama.cpp
- OpenCL: https://github.com/ggml-org/llama.cpp/blob/master/docs/backend/OPENCL.md
- Snapdragon: https://github.com/ggml-org/llama.cpp/blob/master/docs/backend/snapdragon/README.md
- Hexagon developer: https://github.com/ggml-org/llama.cpp/blob/master/docs/backend/snapdragon/developer.md
- Build/KleidiAI: https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md
- Speculative: https://github.com/ggml-org/llama.cpp/blob/master/docs/speculative.md
- IWOCL 2026 (OpenCL на Adreno): https://www.iwocl.org/wp-content/uploads/IWOCL-2026-Wang-Llamacpp.pdf
- Issues: [#19634](https://github.com/ggml-org/llama.cpp/issues/19634), [#29473](https://github.com/ggml-org/llama.cpp/issues/29473), [#31645](https://github.com/ggml-org/llama.cpp/issues/31645), [#31104](https://github.com/ggml-org/llama.cpp/issues/31104), [#31485](https://github.com/ggml-org/llama.cpp/pull/31485), [#30391](https://github.com/ggml-org/llama.cpp/issues/30391), [#29165](https://github.com/ggml-org/llama.cpp/pull/29165), [#29786](https://github.com/ggml-org/llama.cpp/issues/29786), [#30654](https://github.com/ggml-org/llama.cpp/issues/30654), [#30649](https://github.com/ggml-org/llama.cpp/issues/30649), [#30821](https://github.com/ggml-org/llama.cpp/pull/30821), [#27191](https://github.com/ggml-org/llama.cpp/issues/27191)

### Qualcomm

- QAIRT SDK: https://softwarecenter.qualcomm.com/catalog/item/Qualcomm_AI_Runtime_Community
- Gen AI Inference Extensions: https://www.qualcomm.com/developer/software/gen-ai-inference-extensions
- Genie tutorial: https://github.com/qualcomm/ai-hub-apps/blob/main/tutorials/llm_on_genie/README.md
- GenieX: https://github.com/qualcomm/GenieX
- GenieX docs: https://geniex.aihub.qualcomm.com
- GenieX run notes: https://github.com/qualcomm/GenieX/blob/main/notes/run.md
- GenieX release/HTP signing: https://github.com/qualcomm/GenieX/blob/main/notes/release.md
- QNN Maven: https://repo1.maven.org/maven2/com/qualcomm/qti/qnn-runtime/2.48.0/
- QNN LiteRT delegate: https://repo1.maven.org/maven2/com/qualcomm/qti/qnn-litert-delegate/2.48.0/
- GenieX Maven: https://repo1.maven.org/maven2/com/qualcomm/qti/geniex-android/0.10.0/
- QNN HTP docs: https://docs.qualcomm.com/doc/80-63442-10/topic/htp_backend.html
- DSP lib loading: https://docs.qualcomm.com/doc/80-63442-10/topic/dsp_lib_loading.html
- Issues: [ai-hub-apps#157](https://github.com/qualcomm/ai-hub-apps/issues/157), [executorch#21586](https://github.com/pytorch/executorch/issues/21586)

### Модели и спекулятивное декодирование

- Gemma 4 E4B GGUF (unsloth): https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF
- Gemma 4 E4B GGUF (ggml-org): https://huggingface.co/ggml-org/gemma-4-E4B-it-GGUF
- Gemma 4 QAT Q4_0: https://huggingface.co/google/gemma-4-E4B-it-qat-q4_0-gguf
- Ternary-Bonsai: https://huggingface.co/prism-ml/Ternary-Bonsai-2-4B-gguf
- Clover/PR #31306: локальный `external/llama-clover-src` @ 4838ffb

### Внутренние отчёты проекта

- `docs/research/agents/04-gpu-npu.md` — Vulkan/OpenCL/NPU на 845, замеры.
- `docs/research/agents/09-device-bonsai.md` — Bonsai Q2_0 + LoRA на 845.
- `docs/research/agents/10-rust-llama-ffi.md` — Rust ↔ llama.cpp.
- `docs/research/agents/11-rust-android-packaging.md` — APK, 16 KB, размеры.
- `docs/13-CLOVER.md` — Clover 4838ffb, почему не mainline.
- `docs/01-DEVICES.md` — бюджет памяти и устройства.
