# 00 — Передача проекта в следующую сессию

## Главное

- Проект переписывается **с нуля на Rust**. Kotlin/Compose + C++ v0.1 удалён из
  папки и перенесён в архив:
  `C:\OpenCode\_archive\SnapdragonLLM-kotlin-v01-YYYYMMDD-HHMMSS`.
- Модели, документация, исследования и исходники llama.cpp сохранены.
- Требование пользователя: **никакого Kotlin**, всё приложение на Rust (UI +
  логика; llama.cpp — C++-библиотека через FFI).

## Что уже есть

- Rust workspace: `crates/sllm-core`, `crates/sllm-cli`, `crates/sllm-app`.
- `sllm-core`: конфиг, device profile, GGUF preflight, model packs, JSONL-логи.
- Модели: Clover 35B-A3B, Ternary-Bonsai 2-4B Q2_0 + LoRA, Qwen2.5-0.5B.
- `external/llama-clover-src` (PR #31306 @ 4838ffb), `external/llama.cpp`.
- Полные исследования: `docs/research/agents/01…11` + deep-dive.
- UX/IA: `docs/design/2026-11-08-ux-spec.md`.
- План Rust-переписывания: `docs/plans/2026-11-08-rust-rewrite.md`.

### Основное приложение: WebView + Rust (2026-11-09)

- `crates/sllm-webapp` — `MainActivity.java` (Activity + WebView + insets + JNI)
  и Rust `cdylib`. APK собирается `cargo-apk2 1.5.0`.
- `web/` — Preact 10 + Vite 7, Paper & Ink — Editorial. Прод-бандл:
  35 KB JS (13.1 KB gzip) + CSS 54 KB (29.1 KB gzip, includes local fonts).
- Ассеты `web/dist` в APK; отдаются через `shouldInterceptRequest` с
  `https://app.local/` (иначе ES-модули блокируются на `file://`).
- JNI `Sllm.models()` → Rust `sllm-core` сканирует
  `Android/data/dev.snapdragonllm/files/models`, считает preflight и RAM-fit,
  отдаёт JSON. В Models — реальные Clover/Bonsai/Qwen, чипы MoE / Ternary /
  LoRA, Storage-bound.
- IME работает: ввод, отправка, автоскролл; нижняя навигация скрывается при
  фокусе в композере, композер над клавиатурой.
- **APK 3 287 020 B (3.29 MB)** после линковки llama.cpp + `libc++_shared`;
  cold start/PSS на устройстве нужно перемерить (телефон пока не подключён).
- Скриншоты: `docs/design/screens/device-*.png`; токены и компоненты:
  `docs/design/2026-11-09-editorial-ui.md`.
- Запуск UI: `cd web && npm install && npm run dev`; `?frame=phone`,
  `tablet-preview.html`.
- План моделей и движка: `docs/plans/2026-11-09-model-runtime.md`.

### Результаты субагентов (2026-11-10)

- **Motion**: `docs/design/2026-11-10-motion.md` — токены, сниппеты, reduced
  motion, QA. Внедрены токены, убраны повторные stagger-анимации списков,
  убраны blur у nav/шапки.
- **Theme**: `docs/design/2026-11-10-theme.md` — Light/Dark/Night/AMOLED,
  WCAG-таблица, Android-синхронизация. Внедрено: `data-theme`, inline-скрипт
  без вспышки, `Sllm.theme/themePref/setTheme`, перекраска WebView/root и
  иконок статус-бара, переключатель в Settings.
- **Registry**: `crates/sllm-model` + `docs/models/2026-11-10-registry.md`.
  11 тестов, split/LoRA/mmproj/dedupe/fit, JSON camelCase; подключён к
  `sllm-webapp` вместо ручного сканера.
- **FFI**: `crates/sllm-llama-sys` + `docs/plans/2026-11-10-ffi-status.md`.
  Host и Android arm64 собираются; smoke Qwen 0.5B: load 290 ms, decode
  77 ms, greedy token. Пока не слинкован в APK.
- **Engine**: `crates/sllm-engine` + `docs/plans/2026-11-10-engine-contract.md`.
  Trait `Engine`, `MockEngine`, 41 тест, stop/cancel/trim.
- **Ternary/LoRA**: `docs/models/2026-11-10-ternary-lora.md`. PTQ1_0 в
  4838ffb — abort; LoRA rank-1 обучен на PTQ1_0; merge в ternary бессмыслен.
- **Dense/MoE**: `docs/models/2026-11-10-dense-moe.md`. Clover file_type
  врёт (Q4_0 vs IQ3/IQ2), mainline считает иначе; Qwen — golden baseline.

### Fallback: Rust + Slint 1.18.1

- `crates/sllm-app` — прежний shell; APK ~8.6 MB, cold start 204–231 ms,
  PSS 48–55 MB.
- **Известный дефект:** IME открывается, но commit текста не доходит до
  `TextInput` (Slint 1.18.1, Rust 1.98/1.99). Поэтому основной UI переведён
  на WebView.
- Оставлен как лёгкий fallback и для сравнения.

Toolchain: Rust 1.99.1, edition 2024 (актуальная редакция на ноябрь 2026);
сборка также проверена на 1.98.1.

## Устройства

| Роль | Устройство | SoC | RAM | Экран |
| --- | --- | --- | ---: | --- |
| Основное | OnePlus Pad 2 (OPD2403) | Snapdragon 8 Gen 3 | 16 GB | 2120x3000 |
| Ворота | Xiaomi MI 8 (dipper) | Snapdragon 845 | 6 GB | 1080x2248 |

Pad 2: Android 16, dotprod/i8mm/bf16, Adreno 750, Hexagon v75, SDK 36.
MI 8: Android 15, без dotprod/i8mm, Adreno 630, Hexagon 685.

## Что решается дальше

1. **`LlamaEngine` + JNI — сделано (2026-11-11).** `crates/sllm-webapp/src/host.rs`
   держит `Arc<dyn Engine>`, грузит модель в фоне и отдаёт события
   `started/token/done/error/model` через `Sllm.poll()`; `start/cancel/prepare`
   подключены к UI. Mock-стриминг удалён. Runtime LoRA подключён к
   `sllm-llama-sys` (`llama_set_adapters_lora`).
2. **FFI в APK — сделано.** `sllm-llama-sys` → `sllm-engine` → `sllm-webapp`,
   `libc++_shared.so` в APK; release APK собирается. Осталось прогнать на
   устройстве (телефон пока не подключён).
3. **Модели**: Qwen 0.5B и Ternary-Bonsai Q2_0 + LoRA проверены на хосте;
   Clover 11.69 GB загружается (clover_moe/mmap). Дальше — MTP/DFlash/DSpark,
   n-gram speculation, mmproj, OpenCL/Vulkan/QNN.
4. **Планшет**: проверить Light/Dark/Night/AMOLED, rail + two-pane на
   OnePlus Pad 2.
5. **FGS/SAF**: foreground service, импорт моделей, permissions.
6. **Сборки/backends**: baseline armv8-a (845), v86 (8 Gen 3), universal;
   LiteRT и Hexagon — отдельные backend-крейты, пока не подключены.

## Правила

- Не возвращать Kotlin.
- Не тащить CUDA/десктопные решения.
- Числа только с устройства.
- Модели не в APK.
- Старые результаты не переписывать; помечать «not reproduced».

## Бриф для следующей сессии

> Проект SnapdragonLLM в `C:\OpenCode\SnapdragonLLM`. Приложение переписывается
> на Rust; старый Kotlin/C++ в архиве. Прочитай `README.md`, `docs/00-HANDOFF.md`,
> `docs/DECISIONS.md` (ADR-012), `docs/research/`, `docs/plans/`. Начинаем с
> выбора Rust UI-фреймворка и Rust↔llama.cpp FFI, затем главный экран/чаты и
> сборка APK под Pad 2 и MI 8.
