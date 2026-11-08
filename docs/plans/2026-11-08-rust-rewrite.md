# Rust rewrite — план

Дата: 2026-11-08. Заменяет Kotlin/C++ план. Старый код в архиве вне проекта.

> 2026-11-09: основной UI переведён на **Preact + WebView**
> (`crates/sllm-webapp` + `web/`): APK 0.36 MB, cold start 399–550 ms,
> PSS 84–99 MB, IME работает. Slint (`crates/sllm-app`) оставлен как лёгкий
> fallback. Ядро по-прежнему Rust; UI не содержит бизнес-логики.

## Принципы

1. **Никакого Kotlin.** UI и вся логика — Rust. llama.cpp — C++-библиотека через
   собственный FFI-крейт.
2. **Один APK — две формы.** Телефон (bottom nav, один экран) и планшет
   (navigation rail + two-pane) из одного UI-кода.
3. **Главный экран — Chats.** Не пустой чат, не модели.
4. **Модели — данные, не код.** `ModelPack` (base + mmproj + LoRA + license);
   preflight до загрузки.
5. **Один тяжёлый движок за раз.** На 6 GB — 0.5B daily, Clover — demo.
6. **Сначала CPU, потом GPU/NPU.** 845 — только CPU.

## Фазы

### R0 — фундамент (сделано)

- Rust workspace, `sllm-core`.
- GGUF parser/preflight, device profiles, model packs, JSONL, error codes.
- Тесты на Qwen/Bonsai/Clover/LoRA проходят.

### R1 — Rust UI shell + FFI sys (UI-часть сделана)

- UI выбран: **Slint 1.18.1** + `android-activity` 0.6 + `cargo-apk2` 1.5.
  APK собран и проверен на MI 8 (Pad 2 ждёт подключения).
- `sllm-llama-sys`: bindgen + C++ shim + CMake; `llama_process`,
  `llama_batch_ext`, `common_chat_templates`, sampling.
- Minimal app: Chats list + Chat + Models + Settings, Paper & Ink.
- Модели читаются из `Android/data/dev.snapdragonllm/files/models` через
  `sllm-core` в фоновом потоке.
- **Данные (MI 8):** cold start 213–231 ms, PSS ~47–51 MB, APK 8.67 MB,
  IME и композер над клавиатурой работают.
- **Gate:** APK запускается на обоих устройствах, UI не хуже Compose.

### R2 — телефонный UX

- Bottom nav 3 раздела, Chats home, поиск, new chat, чат, composer.
- **Блокер:** IME открывается, но commit текста не доходит до `TextInput`
  (Slint 1.18.1, Rust 1.98/1.99). Исправить до FFI: проверить фокус/дерево,
  затем `LineEdit` в изолированном компоненте, при необходимости —
  `input-type`, `focus()` и Java-хелпер.
- Модели: импорт, preflight, загрузка, ошибки.
- Настройки: Generation, Performance, Storage, Appearance, Privacy.
- **Данные:** 200 сообщений, font scale, IME, predictive back, OOM-путь.
- **Gate:** Qwen 0.5B чат на MI 8 и Pad 2.

### R3 — планшетный UX

- Navigation rail + list/chat two-pane; Models/Settings list-detail.
- Поворот, resize, split-screen, max width 720.
- **Данные:** 1060/1500 dp layouts, draft/scroll сохранение.
- **Gate:** Pad 2 выглядит как native tablet app, не растянутый телефон.

### R4 — движок и модели

- Qwen 0.5B, Bonsai Q2_0 + LoRA, Clover 35B.
- Persistent threadpool, mmap, KV q8_0/q4_0, preflight, memory governor.
- **Данные:** TTFT, prefill, decode, PSS, thermal на Pad 2 и MI 8.
- **Gate:** Qwen ≥40 tok/s на Pad 2; Bonsai ≥8; Clover ≥2 (гипотезы до замера).

### R5 — сборки и backends

- baseline `armv8-a` (845), v86 `armv8.6+dotprod+i8mm+fp16` (8 Gen 3/Elite).
- Universal `GGML_CPU_ALL_VARIANTS` + `BACKEND_DL` — позже.
- OpenCL opt-in на 8 Gen 3 (prefill); Vulkan/Hexagon — после замеров; 845 — CPU.
- **Данные:** CPU/OpenCL/Vulkan/NPU A/B, thermal, stability.
- **Gate:** таблица backend по устройству.

### R6 — мультимодальность по возможностям модели

- В чате — только то, что умеет выбранная модель: text, vision (mmproj), audio,
  video. Никаких отдельных ASR/TTS-модулей как обязательных.
- `ModelPack` companion-файлы; UI показывает capability badges и disabled-пункты
  с причиной.
- Видео — MediaCodec → 8–12 кадров → VLM; аудио — модель с audio projector.
- **Gate:** фото/видео/аудио работают на моделях, которые это поддерживают.

## Первые задачи

1. Дождаться отчёта по Rust UI/APK и языкам 2026.
2. Собрать минимальный Slint APK на Pad 2.
3. Подключить `sllm-llama-sys`, загрузить Qwen 0.5B.
4. Сделать Chats/чат/Models/Settings по UX-спеке.
5. Замерить и зафиксировать baseline.
