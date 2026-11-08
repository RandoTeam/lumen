# Многофазный план SnapdragonLLM (2026-11-08)

## Цель и принципы

Довести приложение до состояния «профессиональный локальный LLM-инструмент»,
который сначала максимально выжимает **Xiaomi MI 8 / SD845 / 6 GB**, а затем
переносится на 8 Gen 3 / 8 Elite без переписывания.

Принципы:

1. **Сначала надёжность, потом фичи.** Красивое приложение, которое теряет чат
   и падает по ANR, не считается продуктом.
2. **Одна тяжёлая модель за раз.** На 6 GB одновременно живёт только один
   крупный движок; остальные выгружаются.
3. **Всё измеряется.** Каждая подфаза имеет данные, метрики и критерий выхода.
4. **Сначала CPU, потом GPU/NPU.** На 845 GPU/NPU уже проверены и отклонены;
   на новых устройствах — после CPU-baseline.
5. **Пакеты способностей.** Текст — ядро; vision/audio/image-gen — отдельные
   `.so` + модели, лениво.
6. **Никаких «улучшений» без quality gate.** 22 промпта + sustained.

## Легенда приоритетов

- **P0** — блокирует продукт/стабильность.
- **P1** — нужно для заявленной функциональности.
- **P2** — исследование/новые устройства.

---

# Phase 0 — Фиксация baseline и сохранение знаний (0.5–1 день)

## 0.1. Сохранить полные отчёты

- Файлы: `docs/research/agents/01…05`, `docs/research/2026-11-07-deep-dive.md`,
  `docs/research/README.md`.
- **Данные:** все пять отчётов, включая 1.3/1.4, сохранены без сокращений.
- **Критерий:** `rg "1.3|1.4|expert-profile|F8_E4M3" docs/research` находит всё.

## 0.2. Зафиксировать текущий baseline

- Собрать release APK, SHA-256, размер; сохранить `build/session.jsonl`,
  `docs/device-845.md`, `dumpsys meminfo`, `thermalservice`.
- **Метрики:** PSS оболочки 60–71 MB; Qwen 25–26 tok/s, 48 prefill; Clover
  0.60–0.67 tok/s; APK 11.4 MB; 16 KB alignment OK.
- **Критерий:** `docs/baseline/2026-11-08.md` с числами и командами.

## 0.3. Регрессионный harness

- Скрипты: `scripts/check-baseline.ps1` — install, cold start, send prompt,
  PSS, tok/s, `zipalign -c -P 16`, `logcat` errors.
- **Данные:** JSONL на прогон: build hash, device, PSS, TTFT, decode, thermal.
- **Критерий:** один прогон < 10 минут, сравнение с baseline.

## 0.4. Инвентаризация устройства

- Модели, свободное место, page cache, zram, thermal, root, ABI.
- **Данные:** `/proc/meminfo`, `df`, `dumpsys meminfo`, `thermalservice`.
- **Критерий:** `scripts/probe-device.ps1` воспроизводит отчёт.

---

# Phase 1 — Надёжность P0 (3–5 дней)

## 1.1. Single-flight движка

- Файлы: `AppController.kt`, `Engine.kt`, `snapcore.cpp`.
- Ввести состояние `EngineState` + `Mutex`/actor: `selectModel` игнорирует
  повторный вызов в `LOADING`; `free` только после завершения загрузки.
- **Данные:** два быстрых тапа → один `nativeLoad`; нет UAF/утечки.
- **Тест:** monkey 100 тапов по Models; `dumpsys meminfo` стабилен.
- **Критерий:** 0 двойных загрузок в логах.

## 1.2. Отмена и trim без блокировки UI

- `nativeCancel` → `std::atomic<bool>`; проверка отмены в prefill-цикле и
  между токенами; `nativeTrim` только после генерации (`pendingTrim`).
- `stop()` не берёт `e->lock` с main-потока.
- **Данные:** Stop latency ≤1 токен/чанк (≤200 ms на prefill-чанк); KV не
  портится.
- **Критерий:** нет ANR; после Stop можно сразу отправить новый запрос.

## 1.3. Foreground service

- `GenerationService` (`specialUse`), уведомление с моделью/токенами/Stop,
  `PARTIAL_WAKE_LOCK` только на генерацию.
- Activity подписывается на `StateFlow`.
- **Данные:** экран off 5 минут → генерация продолжается; процесс не убит.
- **Критерий:** 0 потерянных ответов при background.

## 1.4. Память и жизненный цикл

- `unloadModel`/`updateConfig` — только на IO; после `free` — `malloc_trim`.
- `onTrimMemory`: 20 — пауза polling, 60 — после генерации очистить KV,
  80 — free context, оставить model.
- **Данные:** PSS idle ≤75 MB; после 20 сообщений ≤85 MB.
- **Критерий:** нет LMK за 30 минут чата.

## 1.5. Сохранение чатов

- `chats/<id>.jsonl` + индекс; автосейв после каждого ответа; восстановление.
- **Данные:** kill -9 во время генерации → чат восстановлен без потери
  предыдущих сообщений.
- **Критерий:** 0 потерь; экспорт Markdown/JSON.

## 1.6. Импорт моделей

- `OpenMultipleDocuments`, фильтр `.gguf`; `.part` → fsync → atomic rename;
  SHA-256 потоком; `StatFs` (`size*1.15 + KV + compute`); отмена.
- `GgufMeta` — лимиты на строки/массивы (не 1 GB в `ByteArray`).
- **Данные:** импорт 0.4/4/12 GB; обрыв сети/диска; свободное место.
- **Критерий:** нет битых файлов в списке; ошибка с числами.

## 1.7. Логи и диагностика

- Постраничный просмотр с конца, фильтр `type/run_id`, share файлом через
  `FileProvider`, пауза polling вне экрана.
- Пробрасывать native error string (arch/type/quant) в UI.
- **Данные:** открытие логов ≤150 ms, нет кадров >50 ms.
- **Критерий:** пользователь видит причину, а не `engine load failed`.

## 1.8. Конфиг

- `config.json` — только явный override по кнопке «Apply external config»,
  либо `config.version`; Tuning не перекрывается на каждом старте.
- **Данные:** изменить threads → restart → значение сохранилось.
- **Критерий:** настройки не «откатываются».

**Выход Phase 1:** P0-тесты зелёные, PSS/скорость не хуже baseline >5 %,
ANR/потерь чата нет.

---

# Phase 2 — Совместимость GGUF и модель-менеджер (4–7 дней)

## 2.1. GGUF preflight

- Расширить `GgufMeta`: arch, file_type, block_count, n_expert, ctx_train,
  chat_template, tensor type histogram (TQ/Q2/PQ/F8), mmproj/shards.
- `ModelPack`: `kind`, `files[{role, sha, bytes}]`, `license`, `sourceUrl`.
- **Данные:** 20+ GGUF разных семейств; точность arch/type 100 %.
- **Критерий:** до загрузки — вердикт `works/likely/needs work/unsupported`.

## 2.2. Compatibility gate

- Allowlist архитектур 4838ffb; denylist `F8_E4M3`, arch новее, split-шарды
  без первого, mmproj без vision-пакета.
- Проверка chat template/BOS; override шаблона.
- Оценка памяти: `weights + KV + compute + reserve`; предупреждение для 6 GB.
- **Данные:** таблица «модель → статус → причина» в UI/логах.
- **Критерий:** ни один неподдерживаемый GGUF не доходит до `nativeLoad`.

## 2.3. Менеджер моделей

- Удаление, размер, SHA, свободное место, «влезет/не влезет», профили
  `model+device`, кэш метаданных, запрет автозагрузки >2 GB.
- **Данные:** 10 моделей; скан <100 ms; PSS +≤2 MB.
- **Критерий:** пользователь понимает, что загружено и чего не хватает.

## 2.4. Закрыть 1.3/1.4 полностью

- Vision/audio/embedding/T5/diffusion — явные статусы и отказ с объяснением.
- Split GGUF: выбирать `00001-of-...`, остальные не показывать отдельно.
- **Данные:** тест-матрица из отчёта 01 (tiny/small/MoE/ternary/vision/audio).
- **Критерий:** 1.3/1.4 не имеют открытых пунктов.

**Выход Phase 2:** любой `.gguf` получает preflight-вердикт; менеджер моделей
готов к base+mmproj+LoRA; PSS ≤75 MB.

---

# Phase 3 — Ternary, Bonsai 2.4B и LoRA (5–8 дней) — приоритет пользователя

## 3.1. Аудит ternary-файлов

**Уже проверено (факт, `docs/research/agents/08-bonsai-lora.md`):**

- Это **Bonsai 2 / 4B**, а не 2.4B: база Qwen3.5-4B, `qwen35`, 4.21B, 32 слоя
  (8 full-attention + 24 Gated DeltaNet), **dense, не MoE**.
- Годится **только `Ternary-Bonsai-2-4B-Q2_0.gguf`, 1 212 103 520 B**.
- `PTQ1_0` (type 143) и `PQ2_0` (type 142) — приватные типы PrismML; в 4838ffb
  `GGML_TYPE_COUNT=43`, они не читаются. Не тратить время на патч — берите Q2_0.
- Hadamard v2 для `qwen35` поддержан; модель загружается и генерирует текст.
- Тензоры: 225×F32 (19.6 MB) + 201×Q2_0 (1181.5 MB); 1201 MB данных.

**Работа:** скачать Q2_0, сверить SHA-256, прогнать preflight и CLI.
- **Данные:** load time, prefill/decode, RSS/PSS, thermal.
- **Критерий:** Q2_0 загружается; PTQ1_0/PQ2_0 дают `E_QUANT_UNSUPPORTED`.

## 3.2. BitNet relu2 (отдельная ветка)

- Патч `src/models/bitnet.cpp` + `llama-model.cpp` (activation `relu2`).
- **Данные:** BitNet 2B4T TQ1_0 до/после; качество/скорость.
- **Критерий:** осмысленный текст; иначе BitNet не включать.

## 3.3. Bring-up Bonsai 2-4B

- Скачать `Ternary-Bonsai-2-4B-Q2_0.gguf` (1.21 GB) в `models/ternary/`;
  `Get-FileHash` → `64A5183B…9433D4`.
- CLI: `llama-completion -m … -c 512 -t 4 -n 16 --temp 0`.
**Измерено на MI 8 (факт, `docs/research/agents/09-device-bonsai.md`):**

| Метрика | Q2_0 base | Q2_0 + LoRA |
|---|---:|---:|
| Load | **2.15 s** | ~2.2 s |
| Prefill (8 tok) | **3.73 tok/s** | ~3.7 |
| `llama-bench` pp128 | **3.08 tok/s** | — |
| Decode | **2.55 tok/s** | **2.41 tok/s** |
| `llama-bench` tg32 | **2.32 tok/s** | — |
| RSS | **1.32 GiB** | 1.33 GiB |
| Thermal cpu3-gold | 34.8 → **90.6 °C** | 43.6 → 83.1 °C |
| Battery | 30 → 31 °C | 32 → 33 °C |

Вывод: модель работает, память в норме, но **2.3–2.6 tok/s** — в 10 раз
медленнее Qwen2.5-0.5B (26 tok/s). Для ежедневного чата на 845 это demo, не
daily driver. Основные причины: 4.21B, Hadamard на каждый матмул, нет repack,
4×A75 быстро уходят в троттлинг (90.6 °C за 20–30 с).

- **Данные:** load time, prefill, decode, TTFT, PSS, thermal, battery.
- **Критерий:** модель работает; числа записаны; quality gate 22 промпта.
- **Решение:** держать как «Ternary demo» / тест качества, не как модель по
  умолчанию. Для daily — Qwen2.5-0.5B или 1.5B Q4.

## 3.4. LoRA-движок

**Уже проверено (факт):**

- `AtomicChat/Ternary-Bonsai-2-4B-Uncensored-LoRA-GGUF` — GGUF-адаптер,
  **2 368 992 B**, SHA `605E9DFB…38A9`, Apache-2.0, rank-1, F32, 64 сайта.
- Работает **рантаймом поверх Q2_0**, без merge и без переквантования:
  `llama_adapter_lora_init` + `llama_set_adapters_lora`; KV и контекст не
  сбрасываются; scale 0 = выключено.
- Память +2.37 MB; скорость −0…5 % (в шуме). `token_embd` в адаптере
  запрещён (hadamard latent table).
- Merge в ternary-файл бессмыслен: rank-1 дельта округлится до нуля.

**Работа:**

- JNI: `nativeLoraLoad(handle, path) -> JSON`, `nativeLoraSet(handle, enabled, scale)`.
- `ModelStore`: `general.type=adapter` → адаптер, **не модель**; фильтр в списке.
- `Prefs`/`pack.json`: `{adapterPath, scale, enabled}`; авто-применение при load.
- UI: бейдж **UNCENSORED (abliterated)**, toggle, слайдер scale, age-gate при
  первом включении, кнопка «выгрузить», +2.3 MB RAM.
- `nativeInfo`: `arch`, `quant=Q2_0`, `hadamard=2`, `lora{path,scale,active,sites,mb}`.
**Измерено на MI 8 (факт):**

- LoRA **применяется** поверх Q2_0: scale 0 ≡ base, scale 1 меняет greedy-вывод,
  scale 100 ломает в повтор; неверный путь даёт явную ошибку.
- +4.2 MB RSS, decode 2.55 → 2.41 tok/s (в шуме).
- **Но** адаптер обучен на `PTQ1_0` (sha в метаданных), а мы применяем к `Q2_0`.
  Механически работает, авторских замеров refusal removal на Q2_0 нет.
- Тайминги `common_perf_print` с LoRA ломаются (`load/prompt eval = 0`,
  время уходит в `unaccounted`) — это баг ветки, не «мгновенно».

- **Данные:** apply time, memory delta, decode delta, совместимость с Q2_0.
- **Критерий:** LoRA включается/выключается на лету; ответ меняется; scale 0
  возвращает базу; в логе есть `lora.active` и `scale`.
- **Решение:** включать как experimental, с предупреждением «adapter trained on
  PTQ1_0; applied to Q2_0». Для 100 % авторского поведения нужен PrismML fork
  и PTQ1_0, а не Q2_0.

## 3.5. ModelPack base + LoRA

- Пакет: base + adapter(s) + license; UI: toggle, scale, «uncensored» badge,
  предупреждение, сохранение выбора.
- **Данные:** перезапуск приложения сохраняет LoRA; удаление адаптера не
  ломает базу.
- **Критерий:** пользователь в 2 тапа включает LoRA.

## 3.6. UI ternary

- Бейдж `TERNARY 2-bit`, preset `ctx 2048 / batch 64 / q8_0`, подсказка
  «длинный prefill, короткие диалоги».
- **Данные:** PSS/скорость с пресетом; ошибки на неподдерживаемом файле.
- **Критерий:** нет «молчаливой» деградации.

## 3.7. Quality gate

- 22 промпта: base vs base+LoRA; RU/EN, JSON, tool call, код, длинный контекст.
- **Данные:** pass/fail, flaky, скорость; MMLU-ориентир 68.7 (n=300, ±2.7)
  из карточки адаптера — не гарантия.
- **Критерий:** база проходит согласованный порог; LoRA не ломает язык/код.

## 3.8. Thinking off

- По умолчанию Bonsai уходит в `<think>` и съедает весь `max_tokens`.
- В chat template есть `enable_thinking`; приложение должно ставить
  `enable_thinking=false` для коротких ответов и давать тумблер «Reasoning».
- **Критерий:** короткий prompt получает ответ, а не обрыв в thinking.

## 3.9. Лицензии и безопасность

- `NOTICE.txt` Prism ML + Alibaba Qwen3.5-4B + AtomicChat; Apache-2.0.
- Uncensored: age-gate, дисклеймер, возможность выключить, не включать по
  умолчанию; политика Google Play — отдельно проверить.
- **Критерий:** лицензии в `THIRD_PARTY_NOTICES.md`; UI предупреждает.

**Выход Phase 3:** Bonsai 2-4B Q2_0 работает на 845 (2.3–2.6 tok/s, 1.32 GiB),
LoRA интегрирована как experimental, есть честные числа и quality-отчёт.
Модель помечена «Ternary demo», не default. PTQ1_0/PQ2_0 не поддерживаются
(types 143/142); для них нужен PrismML fork — отдельное решение, не в этой фазе.

---

# Phase 4 — Clover: экспертный кэш и I/O (6–10 дней)

## 4.1. E0b: убрать whole-file prefetch

- `init_mappings` без `MAP_POPULATE`/`WILLNEED` на 12.5 GB; `FADV_RANDOM` на
  экспертные диапазоны.
- **Данные:** load time, page-cache residency, decode.
- **Критерий:** загрузка быстрее/не хуже, hit не хуже.

## 4.2. Новый профиль 40×256

- Снять router ids на 2–4 тыс. токенов; построить `STRP` 40×256; проверить,
  что текущий 48×512 не используется.
- **Данные:** top-16/layer, skew, hit@slots.
- **Критерий:** профиль валиден; hot set не мусорный.

## 4.3. Host-пул экспертов

- Host-режим `llama_moe_cache` или отдельный `moe_io`; 24 слота/слой (966 MiB);
  O_DIRECT pread, 2 потока, 4 KiB bounce; admission 2-hit; prefill bypass.
- **Данные:** hit rate, SSD MB/token, I/O wait, decode, PSS.
- **Критерий:** hit ≥30 %, decode ≥0.85 tok/s, PSS ≤4.3 GB.

## 4.4. Prefetch L+1

- Top-2 `WILLNEED` только idle; включать при precision >30 %.
- **Данные:** hit, лишний I/O, decode.
- **Критерий:** +10 % decode без роста SSD MB/токен >20 %.

## 4.5. Метрики

- JSONL: hits/misses/read_mb/willneed_mb/io_wait_ms/faults; per-layer.
- **Критерий:** можно объяснить каждый tok/s.

## 4.6. Fallback

- Если hit <30 %: admit-all LRU 16 слотов, откат на page cache, пересборка
  профиля; затем dense-оптимизации (output head Q4/Q5, KV q4_0, ctx 2K).
- **Критерий:** Clover ≥0.9 tok/s или официально «demo only».

**Выход Phase 4:** Clover 0.6 → 0.9–1.1 tok/s, стабильно 10 минут, с логами.

---

# Phase 5 — Мультимодальность в текстовом разделе (8–15 дней)

## 5.1. Разделение ядра

- Mainline-сборка для `mtmd` / новых моделей; clover-сборка для Clover;
  `EngineGate` выбирает `.so`.
- **Данные:** размер APK, PSS, cold start.
- **Критерий:** обе сборки сосуществуют без конфликтов символов.

## 5.2. Image input

- `libmtmd` + mmproj (SmolVLM-500M / Moondream2), lazy load, attachments,
  preview, OCR/vision prompt.
- **Данные:** load time, RAM, latency, quality on 20 images.
- **Критерий:** фото в чате работает на 6 GB; модель без vision — понятная ошибка.

## 5.3. Video input

- MediaCodec: 8–16 кадров 384–512 px → VLM; прогресс; лимит памяти.
- **Данные:** decode time, frames, RAM, quality.
- **Критерий:** 10-секундное видео обрабатывается без OOM.

## 5.4. Audio input

- whisper.cpp tiny/base, 30-секундные чанки, transcript → chat.
- **Данные:** WER на 10 фразах, latency, RAM.
- **Критерий:** русская/английская речь → текст, без сети.

## 5.5. TTS

- Android TTS offline; play per message; voice/language.
- **Данные:** latency, качество, 0 MB модели.
- **Критерий:** кнопка «озвучить» работает офлайн.

## 5.6. Image generation (Studio)

- stable-diffusion.cpp SD1.5 Q4 + TAESD; на 845 — demo, на 16/24 GB — real.
- **Данные:** steps/sec, RAM, качество; выгрузка текстовой модели.
- **Критерий:** 512×512 без OOM; очередь задач.

## 5.7. UX

- Chat «+»: фото/камера/файл/аудио; capability badges; Studio/Voice sections;
  прогресс/отмена.
- **Данные:** клики до результата; ошибки памяти.
- **Критерий:** пользователь не думает о движках.

**Выход Phase 5:** текст + фото/видео/аудио в чате, TTS, Studio (demo/real).

---

# Phase 6 — Бэкенды и новые устройства (5–10 дней)

## 6.1. CPU-тюнинг 845

- Q4_0 vs Q4_K_M, affinity `cpu4-7`, batch/prefill chunk, KV q4_0; repack
  реально не активен — не обещать.
- **Данные:** decode/prefill, thermal, quality.
- **Критерий:** +10–20 % без потери gate.

## 6.2. 845 — зафиксировать CPU-only

- Vulkan не работает (16-bit storage → SIGSEGV), OpenCL 12–14× медленнее,
  NPU v65 не поддержан.
- **Критерий:** в конфиге по умолчанию `gpu_layers=0`, `backend=cpu`.

## 6.3. 8 Gen 3 / 8 Elite

- CPU baseline → OpenCL (prefill) → Hexagon → Vulkan (последним).
- **Данные:** pp/tg, thermal, stability 10 минут.
- **Критерий:** таблица «backend по устройству».

## 6.4. Clover на 16/24 GB

- Бюджет RAM, кэш, ctx; 8 Gen 3 16 GB — на грани; 8 Elite 24 GB — комфортно.
- **Критерий:** Clover ≥5–10 tok/s на 24 GB.

---

# Phase 7 — Релизное качество (постоянно)

## 7.1. Quality gate

- 22 промпта + мобильные: tool call, JSON, RU/EN, needle, отказ от галлюцинации.
- **Критерий:** нет регрессии >5 %.

## 7.2. Sustained

- 30 минут: tok/s, thermal, PSS, LMK; деградация ≤20 %.
- **Критерий:** отчёт с медианой/min/max.

## 7.3. CI

- Build debug/release, lint, unit, zipalign 16 KB, PSS-скрипт, APK SHA.
- **Критерий:** красный CI блокирует релиз.

## 7.4. Лицензии и приватность

- `THIRD_PARTY_NOTICES`, base+LoRA licenses, no network by default, prompts not
  logged, chats in internal storage.
- **Критерий:** юридически чисто; пользователь понимает, что уходит наружу.

---

# Сводная таблица фаз

| Phase | Срок | Главный результат | Ключевые данные | Gate |
| --- | --- | --- | --- | --- |
| 0 | 0.5–1 д | baseline + отчёты | PSS, tok/s, APK, device | воспроизводимость |
| 1 | 3–5 д | P0-надёжность | Stop latency, PSS, 0 потерь | нет ANR/LMK |
| 2 | 4–7 д | compatibility gate | arch/type/table, память | любой GGUF preflight |
| 3 | 5–8 д | Bonsai 2.4B + LoRA | tok/s, PSS, LoRA delta, gate | модель работает |
| 4 | 6–10 д | Clover I/O | hit rate, decode, SSD MB | ≥0.9 tok/s |
| 5 | 8–15 д | фото/видео/аудио/TTS | PSS, latency, quality | 6 GB без OOM |
| 6 | 5–10 д | backend matrix | pp/tg, thermal | CPU default 845 |
| 7 | постоянно | релиз | gate, sustained | RC |

# Первые 10 задач (можно начинать сразу)

1. Сохранить отчёты и baseline (Phase 0).
2. Single-flight + atomic cancel + trim (1.1–1.2).
3. Foreground service (1.3).
4. Импорт `.part` + SHA + StatFs (1.6).
5. Сохранение чатов (1.5).
6. GGUF preflight + ModelPack (2.1).
7. Compatibility gate + ошибки (2.2, 1.7).
8. Bonsai 2.4B: preflight, download, CLI (3.1–3.3).
9. LoRA: формат, apply, UX (3.4–3.5).
10. Clover E0b + новый профиль 40×256 (4.1–4.2).

# Дополнение A. Совместимость: коды, preflight, ModelPack

Основано на `docs/research/agents/06-compat-implementation.md`.

## A.1. Что закрыть в Phase 1–2

| Код | Проблема | Работа |
| --- | --- | --- |
| `E_LOAD_NATIVE` | причина загрузки теряется | ring-buffer логов + `nativeLoadEx` JSON `{ok,handle,error{code,message,detail,log_tail}}` |
| `E_PREFLIGHT_BLOCKED` | autoload неподходящей модели | preflight до `engine.load`; autoload только `chat-capable && preflight.ok` |
| `E_GGUF_TRUNCATED` / `E_QUANT_UNSUPPORTED` | битый/FP8/новый тип | проверка magic/version/offset/type ids до native |
| `E_ARCH_UNKNOWN` | arch новее 4838ffb | allowlist 4838ffb + понятный отказ |
| `E_SPLIT_NOT_FIRST` | выбрали не первый шард | группировка `split.count/no`, грузить с первым |
| `E_MMPROJ_REQUIRED` | mmproj как чат-модель | скрыть mmproj, показать как companion |
| `E_TEMPLATE_FALLBACK` | нет chat template | читать `tokenizer.chat_template`, warning/override |
| `E_RAM_LOW` | 32K ctx / 35B на 6 GB | оценка `weights+KV+scratch+400 MB` |
| `E_CTX_CLAMPED` | ctx больше обучения/бюджета | `min(ctx_train, budget)` + сообщение |
| `E_KV_UNSUPPORTED` | неизвестный KV | allowlist f16/q8_0/q4_0 |
| `E_HISTORY_RESET` | trim очистил native-историю | не чистить без флага; UI-баннер |
| `E_LORA_UNSUPPORTED` | LoRA не применена | проверить adapter/quant; понятный отказ |

## A.2. Preflight (обязательный)

`PreflightReport`: `ok/code/message/detail/warnings/arch/fileType/quant/nLayer/nCtxTrain/
nEmbd/nHeadKv/splitCount/splitNo/shards/mmproj/caps/chatTemplate/license/estimateMb`.

Проверки: `GGUF` magic, version 2–3, offsets, tensor types ∈ 0..42, `general.architecture`,
`general.file_type`, `split.*`, `tokenizer.chat_template`, `clip.*`, `*.head_count_kv`,
`*.context_length`, license. Для MoE отдельно писать «эксперты читаются с UFS».

Оценка памяти:
`weights ≈ file bytes` (mmap, file-backed), `KV = n_layer * n_ctx * n_head_kv * head_dim *
2 * bpe` (f16 2.0, q8_0 1.0625, q4_0 0.5625), `scratch ≈ n_batch * n_embd * 8 * 4`,
`+400 MB` оболочки.

## A.3. ModelPack

```
models/<id>/pack.json
  base:    {file, shards, sha256, bytes}
  mmproj:  {file, sha256, required}
  lora:    [{file, scale, sha256}]
  template: optional
  license: {spdx, name, url}
  capabilities: [text, vision, audio, video, embedding, ...]
  recommended: {ctx, batch, kv}
```

Импорт `.part` → SHA → atomic rename; `StatFs`; отмена; не перезаписывать без
подтверждения; flat GGUF мигрирует в pack.

## A.4. Тест-матрица (14 кейсов)

tiny/chat, 3–4B/trim, MoE/Clover, BitNet TQ, VLM+mmproj, Whisper/audio, MP4,
embedding, T5, diffusion, TTS, split 2-part, FP8, truncated. Для каждого —
`error_code`, `log_tail`, `arch`, `file_type`, `ram_est_mb` в JSONL.

---

# Дополнение B. Мультимодальность: mtmd уже есть

Основано на `docs/research/agents/07-multimodal.md`.

## B.1. Главный факт

В `external/llama-clover-src` **уже есть `tools/mtmd`** (image/audio/video, SmolVLM,
Qwen2.5-VL, Gemma3, Moondream2, Qwen3-ASR). Не нужен mainline ради image input.
Достаточно:

```cmake
set(LLAMA_BUILD_MTMD ON CACHE BOOL "" FORCE)
target_link_libraries(snapcore PRIVATE llama llama-common mtmd android log)
```

`MTMD_VIDEO` при `LLAMA_SUBPROCESS=OFF` выключен — видео делаем через MediaCodec
(8–12 кадров → mtmd), без ffmpeg.

## B.2. Пакеты под 6 GB

| Пакет | Размер | Вердикт |
| --- | ---: | --- |
| Qwen2.5-0.5B Q4_K_M | 0.4 GB | текст, 25 tok/s |
| SmolVLM-256M/500M + mmproj | 0.28–0.55 GB | **default vision** |
| SmolVLM2-500M-Video + mmproj | 0.55 GB | default video |
| Qwen3-ASR-0.6B + mmproj | 1.02 GB | **default ASR** |
| Whisper tiny/base | 75/142 MB | lite/fallback |
| Qwen2.5-VL-3B | 2.77 GB | 8+ GB, experimental |
| Gemma3-4B | 3.34 GB | 8+ GB |
| SD1.5 Q4 / SD-Turbo | 1.57/1.6 GB | 845 demo / 16 GB |

Бюджет приложения на 6 GB: **soft 2.4 / hard 2.8 GB**. Одна тяжёлая модель за раз.
Qwen2.5-VL-3B + 8 кадров = 3.5–5 GB — не грузить.

## B.3. Фазы мультимодальности

- **B1** — `LLAMA_BUILD_MTMD=ON`, smoke-test на устройстве.
- **B2** — image input: SmolVLM-500M, attachments, encode/prefill, качество.
- **B3** — audio input: Qwen3-ASR-0.6B + AudioRecord/MediaCodec.
- **B4** — TTS: Android TTS, voice discovery, offline-check.
- **B5** — video input: 8–12 кадров, SmolVLM2-500M-Video, token cap.
- **B6** — Studio: SD1.5 Q4/SD-Turbo, lazy `.so`, 845 demo.

## B.4. UX

Пять разделов: **Chat · Studio · Voice · Models · Dev** (Tuning/Telemetry/Logs).
В Chat «+» = фото/камера/файл/аудио. Недоступный пункт объясняет причину и даёт
CTA. Вложения — chips с превью; в stats — `encode_ms`, `prefill`, `decode`, `peak`.
Состояния: `MODEL_TEXT_ONLY`, `MMPROJ_MISSING`, `MMPROJ_INCOMPATIBLE`,
`INSUFFICIENT_MEMORY`, `ENCODING`, `PREFILL`, `GENERATING`, `THERMAL_PAUSE`.

---

# Риски

- 6 GB LMK при 12 GB Clover и при одновременной мультимодальности.
- Текущий `expert-profile.bin` не Clover — без нового профиля кэш бесполезен.
- LoRA может требовать dequant/merge и не работать поверх TQ/Q2 без патча.
- `mtmd` может требовать mainline, что конфликтует с clover-форком.
- iOS/Android драйверы Adreno нестабильны; не обещать GPU-буст без замера.
- Лицензии community-моделей/LoRA могут быть неясны.
