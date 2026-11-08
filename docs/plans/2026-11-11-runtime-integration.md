# Реальная интеграция движка (2026-11-11)

## Что было сломано

В Rust-версии `sllm-engine::LlamaEngine` и `sllm-llama-sys` существовали, но
APK их не использовал:

- `MainActivity.Bridge.engine()` возвращал `"mock"`;
- в JNI не было `start/poll/cancel/stats/prepare`;
- WebView не получал токены и не мог запустить модель;
- кнопка «Set active» только меняла состояние React, но ничего не грузила;
- часть кнопок (import, attach, voice, chat options, telemetry) не имела
  обработчиков.

## Что сделано

### Rust

`crates/sllm-webapp/src/host.rs` — единственный владелец движка:

- `OnceLock<Mutex<Host>>` с `Arc<dyn Engine>` (`LlamaEngine`);
- `start(requestJson, modelsRoot)` — разбирает запрос, находит пак в
  `ModelRegistry`, запускает загрузку и генерацию в отдельном потоке и сразу
  возвращает `{ok, generationId, modelId, mock:false}`;
- `poll()` — отдаёт по одному JSON-событию `started/token/done/error/model`;
- `cancel()` — атомарный флаг + `Engine::cancel`, `done.canceled=true`;
- `prepare(modelId, modelPath, root)` — загрузка без генерации для кнопки
  «Set active», прогресс идёт событиями `model: loading/ready/error`;
- `unload()` и `trim(level)` — освобождение весов и Android `onTrimMemory`;
- модель ищется по абсолютному `modelPath` или по id в реестре; при наличии
  LoRA-файла в паке он подключается runtime-адаптером.

`crates/sllm-llama-sys`:

- добавлен `sllm_context_apply_lora()` → `llama_adapter_lora_init` +
  `llama_set_adapters_lora`; адаптеры живут вместе с контекстом;
- Rust API: `Context::apply_lora(path, scale)`.

`crates/sllm-engine::LlamaEngine`:

- `ModelRef.loras` больше не отклоняется: адаптеры применяются при создании
  контекста и повторно после `trim(RunningLow)`;
- проверяются путь и масштаб адаптера.

### Java

`MainActivity`:

- JNI: `nativeEngine`, `nativeStart`, `nativePoll`, `nativeCancel`,
  `nativeStats`, `nativePrepare`, `nativeUnload`, `nativeTrim`;
- `Bridge` реализует `engine/start/poll/cancel/reset/stats/prepare/unload`;
- `onTrimMemory` маппится в `moderate/running_low/complete`;
- `onDestroy` отменяет генерацию и освобождает движок и smoke-модель.

### Web

- `native.ts`: `pollNativeEvent`, `prepareNativeModel`, `llamaVersion`,
  абсолютный `modelPath` из `root + basePath`;
- `App`: таймер 40 мс забирает события из Rust; `done` показывает реальные
  tok/s и TTFT; `error` показывает код и сообщение llama.cpp;
- «Set active» вызывает `prepare` и показывает `Loading… / Active / Load failed`;
- убраны кнопки без реализации: import, attach, voice, chat options,
  telemetry; строки Settings больше не притворяются кликабельными;
- при пустом реестре UI показывает 0 моделей, а не демо-список.

## Проверено

Host, Windows/MSVC, `external/llama-clover-src`:

```text
cargo test -p sllm-webapp --lib                      6 passed
  Qwen2.5-0.5B Q4_K_M: load + 8 токенов
  Ternary-Bonsai Q2_0 + Uncensored LoRA: load + 8 токенов
  Clover APEX-I-NanoPlus V2.1 (11.69 GB): clover_moe/mmap load
cargo test -p sllm-engine --features llama --test llama_golden   2 passed
```

Android release APK собран `cargo-apk2`:

```text
target/release/apk/snapdragonllm.apk
lib/arm64-v8a/libsnapdragonllmweb.so   (llama.cpp + shim + engine)
lib/arm64-v8a/libc++_shared.so
```

`llvm-nm` подтверждает `llama_model_load_from_file`, `llama_adapter_lora_init`,
`llama_sampler_sample`, `sllm_model_load` и экспорты
`Java_dev_snapdragonllm_MainActivity_nativeStart/nativePoll/nativePrepare`.

Не проверено: запуск APK на MI 8 и Pad 2 — телефон пока не подключён.

## Что ещё НЕ реализовано (честно)

Это не «сделано», пока нет кода и проверки:

- **LiteRT / QNN / Hexagon**: в репозитории нет LiteRT SDK и `.litertlm`
  моделей; нужен отдельный backend-крейт и линковка `libLiteRt`.
- **MTP draft**: форк содержит `LLAMA_CONTEXT_TYPE_MTP` и
  `common_speculative`, но наш FFI не подключает `common_params`/`common_batch`.
  Нужен отдельный C++-адаптер спекулятивного цикла.
- **DFlash / DSpark**: то же; в форке есть `arch=dflash`, но нет нашего API
  для draft-модели и block-diffusion проверки.
- **mmproj/vision**: `ModelRef.mmproj` всё ещё отклоняется движком.
- **OpenCL/Vulkan/Hexagon**: сборка CPU-only.
- **SAF-импорт, FGS, настройки генерации в UI**: не подключены.

Эти пункты нельзя закрыть «заглушкой»: пользователь увидит ошибку, а не
работающую функцию. Следующий шаг — либо отдельный backend-крейт LiteRT, либо
C++-адаптер `common_speculative` для MTP/DFlash, с тестами на устройстве.
