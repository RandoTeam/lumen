# Research archive

Полные отчёты исследований от 2026-11-07. Не редактировать числа задним числом:
новые результаты добавлять отдельными файлами/секциями.

| Файл | Что внутри |
| --- | --- |
| `2026-11-07-deep-dive.md` | сводка: совместимость, ternary, I/O, GPU, продукт, рекомендации |
| `agents/01-gguf-compatibility.md` | полная матрица GGUF, ограничения APK, **пункты 1.3 и 1.4**, лестница тестов |
| `agents/02-ternary.md` | TQ1_0/TQ2_0, BitNet relu2, кандидаты, план ternary |
| `agents/03-expert-cache-io.md` | формат `expert-profile.bin`, измерения UFS, бюджет, дизайны A/B/C |
| `agents/04-gpu-npu.md` | Vulkan/OpenCL/NPU на 845 и новых Snapdragon, toolchain, матрица |
| `agents/05-product-audit.md` | P0/P1 аудит приложения, мультимодальность, roadmap, метрики |
| `agents/06-compat-implementation.md` | полный чек-лист 1.3/1.4, коды ошибок, preflight, ModelPack, тест-матрица |
| `agents/07-multimodal.md` | mtmd в 4838ffb, пакеты способностей, vision/audio/video/TTS/SD, бюджеты |
| `agents/08-bonsai-lora.md` | Bonsai 2-4B Q2_0, PTQ/PQ типы, hadamard, LoRA 2.37 MB, JNI/UX |
| `agents/09-device-bonsai.md` | device-тест Bonsai Q2_0 + LoRA на MI 8: 2.3–2.6 tok/s, 1.32 GiB, 90.6 °C |

## Важные предупреждения

1. `models/clover/expert-profile.bin` — **не Clover**: 48×512 = 24 576, у Clover 40×256 = 10 240. Не применять без перегенерации.
2. `moe_cache_mb > 0` на CPU не «no-op», а исключение `MoE cache requires a GPU backend`. В приложении держать 0.
3. APK привязан к 4838ffb. Новые архитектуры mainline и тип `F8_E4M3` не поддерживаются.
4. На 845 Vulkan не работает (16-bit storage → SIGSEGV после патча), OpenCL в 12–14 раз медленнее CPU, NPU v65 не поддержан.
5. Ошибки native-загрузки сейчас теряются: UI показывает только `engine load failed`.

## Как пользоваться

- Перед любым изменением — зафиксировать baseline (`docs/00-HANDOFF.md`, `build/session.jsonl`).
- Для новой модели сначала прогнать preflight GGUF: arch, file_type, типы тензоров, chat template, mmproj/shards.
- Любое «ускорение» проверять quality gate (22 промпта) и sustained 10–30 минут.
