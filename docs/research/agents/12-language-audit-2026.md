# Отчёт 12. Языки и UI-стек для Android в ноябре 2026

Источник: аудит рынка, 2026-11-08. Вопрос пользователя: «Rust 2024 — это старый
язык? Какие есть новые языки и решения?»

## TL;DR

- **«Rust 2024» — это edition, а не год.** Стабилизирован 20.02.2025 в Rust 1.85;
  на ноябрь 2026 остаётся актуальным. Следующий — Rust 2027, ещё не стабилен.
  Текущий stable: **Rust 1.99.1 (15.10.2026)**. Проект обновлён до 1.99.1.
- **Лучший production-путь для Android-LLM — Kotlin 2.4 + Compose + Rust-ядро**,
  но Kotlin запрещён пользователем.
- **Лучший путь без Kotlin — Rust 1.99 + Slint 1.18.1** (проверено APK на MI 8).
  Второй — Flutter 3.47 + Rust-ядро.
- Zig 0.17, Mojo 1.0, Carbon 0.1, Swift 6.4 не дают production-пути к нативному
  Android UI. Это языки ядра/FFI, а не UI.
- «Самый новый язык» не ускоряет LLM: узкое место — бэкенд (CPU/OpenCL/NPU),
  а не синтаксис. Смена языка ядра даёт −6…12 месяцев к MVP.

## Матрица пригодности (1–5)

| Вариант | UI/IME/a11y | Perf/APK | FFI | Eco | MVP | Новизна |
|---|---:|---:|---:|---:|---:|---:|
| Kotlin 2.4 + Compose | 5 | 4 | 4 | 5 | 5 | 2 |
| Kotlin + Compose Multiplatform 1.10 | 4 | 4 | 4 | 4 | 4 | 3 |
| **Rust 1.99 + Slint 1.18** | 3* | 5 | 5 | 3 | 3 | 4 |
| Rust + egui/eframe | 2 | 5 | 5 | 3 | 3 | 4 |
| Rust + Dioxus 0.7 | 2 | 4 | 5 | 3 | 3 | 4 |
| Rust + Iced 0.14 | 1 | 5 | 5 | 3 | 2 | 3 |
| Rust + Makepad 2.0 | 2 | 5 | 5 | 2 | 2 | 4 |
| Swift 6.4 + Android SDK | 2 | 5 | 4 | 3 | 2 | 4 |
| Zig 0.17.1 | 1 | 5 | 5 | 2 | 2 | 5 |
| Mojo 1.0 | 1 | 4 | 2 | 2 | 1 | 5 |
| Carbon 0.1 | 1 | 4 | 3 | 1 | 1 | 5 |
| C++26 + Qt 6.12 LTS | 4 | 4 | 5 | 4 | 3 | 3 |
| C++26 + SDL3 | 1 | 5 | 5 | 3 | 1 | 3 |
| Dart/Flutter 3.47 | 4 | 4 | 4 | 5 | 4 | 3 |
| Go 1.27 + Gio/Fyne | 2 | 4 | 3 | 3 | 3 | 4 |
| React Native 0.85 / Lynx 3.7 | 4 | 3 | 3 | 5 | 3 | 3 |
| Tauri 2 / Capacitor | 1 (WebView) | 4 | 5 | 3 | 2 | 3 |
| C# .NET 11 MAUI | 3 | 3 | 3 | 4 | 2 | 3 |
| C# Avalonia 12 | 3 | 3 | 3 | 4 | 2 | 3 |
| Hare/Odin/Jai/Roc/Vale/Gleam/Elixir | 1 | — | 3 | 1 | 1 | 5 |

\* Slint: IME и AccessKit есть, но Android-a11y и predictive back надо проверять
спайком. Лицензия: GPLv3 / Royalty-free 2.0 с атрибуцией / commercial.

## Ключевые факты по вариантам

- **Rust UI.** Slint 1.18 — единственный Rust-UI с документированным Android для
  Rust: `android-activity`, `cargo-apk2`/`xbuild`, нативный `InputConnection`,
  safe-area/IME, `OnBackInvokedCallback`. Dioxus mobile по умолчанию — WebView.
  Iced официально mobile не поддерживает. egui — официальный Android, но
  a11y/rich text слабее. Freya: «IME not implemented». Xilem — эксперимент.
- **Zig 0.17.1** (02.11.2026), pre-1.0, ломающие изменения. `zig cc` отлично
  собирает C/C++ под NDK, но Android UI нет (Capy — WIP).
- **Mojo 1.0** (11.08.2026), open source, Qualcomm; заточен под AI-ядра/GPU,
  официального Android UI нет.
- **Carbon 0.1** — MVP «не раньше конца 2026», 1.0 после 2028, Android UI нет.
- **Swift 6.4**: официальный Swift SDK for Android (с 6.3), но SwiftUI на
  Android нет — UI всё равно нужен Kotlin/Compose или свой рендер.
- **Kotlin 2.4 / Compose**: лучший IME, TalkBack, predictive back, insets,
  two-pane. Единственный вариант, где всё работает «из коробки».
- **Qt 6.12 LTS**: зрелый Android, но свой рендер, большой APK, LGPLv3/GPLv3/
  commercial.
- **Flutter 3.47**: зрелый не-WebView UI, сильные планшеты и a11y, но
  Service/SAF/IME — через platform channels (Kotlin-клей).
- **React Native/Lynx**: нативные компоненты, но JS-слой; Tauri/Capacitor —
  WebView, что противоречит требованию native.
- **.NET MAUI/Avalonia**: Android-часть слабее, APK тяжелее.

## Что это значит для SnapdragonLLM

1. Ядро — Rust 1.99 / edition 2024, не переписывать. FFI к llama.cpp через
   собственный `sllm-llama-sys` (bindgen + C++ shim + CMake).
2. UI — Slint 1.18.1: проверено на MI 8 (APK 8.67 MB, cold start ~220 ms,
   IME, insets, модель-скан). Планшет — проверить two-pane на Pad 2.
3. «Ноль JVM» на Android недостижим: foreground service, SAF и runtime
   permissions требуют тонкого Java-класса (не Kotlin). Slint уже использует
   Java-хелпер для IME/insets/back.
4. Если Slint упрётся в accessibility/rich text — fallback Flutter + Rust-ядро,
   а не Zig/Mojo/Carbon/Swift.
5. Не тратить время на смену языка UI до MVP: выигрыш нулевой, риск огромный.

## Источники (даты)

Rust 1.99.1 (15.10.2026), Rust 2024 (20.02.2025), Zig 0.17.1 (02.11.2026),
Mojo open source (18.08.2026), Swift 6.4 и Android workgroup, Kotlin 2.4.21,
Compose Multiplatform 1.10, C++26 (28.03.2026), Qt 6.12 LTS (30.09.2026),
Flutter 3.47.9 (29.10.2026), Go 1.27, React Native 0.85 (07.04.2026),
.NET 11 (10.11.2026), Avalonia 12.1 (09.07.2026), Android 16 KB pages
(дедлайн 01.02.2027).
