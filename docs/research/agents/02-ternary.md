# Отчёт 02. Тернарная ветка (полная версия)

Источник: параллельное исследование проекта SnapdragonLLM, 2026-11-07.
Дерево: `external/llama-clover-src` (4838ffb). Устройство: MI 8, SD845, 6 GB.

## TL;DR

- **TQ1_0/TQ2_0 в дереве полностью живые**: dequant + `vec_dot` для CPU, включая ARM NEON **без dotprod** (через `vmlal_s8`), плюс Vulkan-ядра. Repack-пути для TQ нет, активации квантуются в `q8_K`, то есть это всё равно int8-dot, а не «только сложения и сдвиги».
- **BitNet-загрузчик есть** (`arch = "bitnet"`), но у него **баг: FFN считается через SiLU**, а официальная BitNet b1.58 2B4T обучена с `relu2`. Это подтверждено закрытым без мержа PR ggml-org/llama.cpp#26751. Любые замеры качества BitNet в этом дереве будут невалидны без патча.
- **Официальный GGUF Microsoft не загрузится**: arch `bitnet-b1.58` (в дереве только `bitnet`) и тип тензоров 36, который в llama.cpp = удалённый `IQ4_NL_4_4`. Falcon3-1.58bit GGUF — та же история (тип 36, arch `llama`).
- **Рабочий кандидат №1 для bring-up**: `ewchampion/Ternary-Bonsai-1.7B-TQ2_0-GGUF` — 625 MB, arch `qwen3`, `file_type=37`, 196 тензоров TQ2_0, Apache-2.0, без hadamard. Проверено по заголовку файла.
- **Оценка на 845**: 1.7B TQ2_0 ≈ 13–18 tok/s decode, но prefill ≈ 12–18 tok/s (2k промпт — 2–3 минуты). Это модель «для коротких диалогов», не для длинного контекста.

## Что уже есть в движке

**Типы и раскладка.** `GGML_TYPE_TQ1_0 = 34`, `TQ2_0 = 35` (`ggml/include/ggml.h:424-425`), `Q2_0 = 42` там же. Блоки — `block_tq1_0` (54 байта / 256 весов, 1.6875 bpw) и `block_tq2_0` (66 байт / 256, 2.0625 bpw) в `ggml/src/ggml-common.h`. Тип-трейты с `to_float`/`from_float_ref` — `ggml/src/ggml.c:920-935`; `ggml_quantize_chunk` знает оба типа (`ggml.c:8192-8193`), значит `llama-quantize --TQ1_0/--TQ2_0` работает.

**CPU-ядра.** `ggml/src/ggml-cpu/ggml-cpu.c:401-410`: `vec_dot = ggml_vec_dot_tq*_q8_K`, `vec_dot_type = Q8_K`, `nrows = 1`. Реализация в `ggml/src/ggml-cpu/arch/arm/quants.c`:
- TQ1_0 — строки 1400–1575, TQ2_0 — 1577–1686;
- при `__ARM_FEATURE_DOTPROD` — `vdotq_s32` (1464–1474, 1631–1639);
- **без dotprod — `vmlal_s8`** (1476–1495, 1641–1657), то есть widening int8 multiply-accumulate. Это работает на A75/A55, но это именно умножения, а не сдвиги;
- распаковка троичных кодов — сдвиги/AND/умножения на 3/9/27/81 (для TQ1_0) или `vshr+vand` (TQ2_0), то есть на каждые 128 весов ~30 ALU-операций плюс 16 `vmlal`.

**Repack.** В `ggml/src/ggml-cpu/repack.cpp` и `arch/arm/repack.cpp` TQ нет. Значит нет 4×4/8×8 ускорения, которое получают Q4_0/Q8_0; GEMV идёт по одной строке. Для decode это терпимо, для prefill — плохо.

**BitNet-загрузчик.** `src/llama-arch.cpp:97` (`"bitnet"`), `src/llama-model.cpp:227`. `src/models/bitnet.cpp`: читает только RMS eps и `block_count` (26 → `LLM_TYPE_3B`, иначе UNKNOWN — для 2B4T с 30 слоями будет просто UNKNOWN, это только ярлык), тензоры: `attn_sub_norm`, `ffn_sub_norm`, опциональные `*_s` (per-tensor scale официальной модели не нужны), tied embedding используется как lm_head. Граф — `build_ffn(..., LLM_FFN_SILU, LLM_FFN_PAR)` на строке 132.

**Два блокера BitNet:**
1. `LLM_FFN_SILU` вместо `relu2`. В `src/llama-model.cpp:1122-1136` в мапе активаций **вообще нет** `relu2`. PR #26751 (август 2026) это чинил и был закрыт без мержа; в описании прямо сказано: «producing wrong-but-finite logits on every backend».
2. Официальный `microsoft/bitnet-b1.58-2B-4T-gguf` (`ggml-model-i2_s.gguf`, 1.19 GB, MIT) имеет arch `bitnet-b1.58` и 210 тензоров типа **36** (в llama.cpp это «REMOVED» запись с `blck_size = 0`). Я распарсил заголовок: `general.file_type = 40`, `block_count = 30`, `n_embd = 2560`, `n_ff = 6912`, `vocab = 128256`. Загрузка упадёт на неизвестной архитектуре и на типе 36.

**Квантайзер и imatrix.** `tools/quantize/quantize.cpp:48-49` даёт `TQ1_0`/`TQ2_0`. `src/llama-quant.cpp:445-446` — фолбэк на Q4_0 при несовпадении размеров, `:578` — token_embd при TQ-ftype принудительно Q4_K, `:947-948` — ftype. Но `quantize_tq1_0/2_0` в `ggml/src/ggml-quants.c:2415-2427` **игнорируют imatrix** (`(void) quant_weights`), а сама тернаризация (`:2317-2381`) — это `lroundf(x / absmax)` на блок 256, без порога absmean, без группы и без zero-point. Отсюда: обычную модель в TQ конвертировать нельзя, получится мусор хуже Q2_K; а вот **уже тернарную** модель конвертировать можно почти без потерь, потому что при `x ∈ {-s,0,+s}` absmax блока = s и trits восстанавливаются точно.

**Конвертера BitNet в этом дереве нет**: в `convert_hf_to_gguf.py` нет ни одного упоминания BitNet (только `gguf-py/gguf/constants.py` знает arch). То есть путь «safetensors → f16 GGUF → TQ» придётся делать чужим скриптом.

**GPU.** В дереве есть Vulkan-путь: `ggml-vulkan.cpp:2179` (allowlist), `:2946-2947` (`mul_mat_vec_tq1_0/tq2_0`), `:3100-3101` (dequant), `:3130-3131` (get_rows). В OpenCL TQ **нет**. APK собирается без Vulkan/OpenCL (`app/src/main/cpp/CMakeLists.txt:28-33`, `gpuLayers=0`), так что на 845 практический путь — только CPU. `GGML_NATIVE=OFF` означает сборку под `armv8-a` без `+dotprod`: на 845 это правильно (нет SIGILL), но и на 8 Gen 3 dotprod не включится.

## Что реально даст на 845

Плюсы: 1.7B-класс в 0.6 GB, mmap, KV q8_0 4k ≈ 60 KB/токен × 28 слоёв ≈ 0.24 GB, полный RSS ~1.0–1.2 GB — влезает в 6 GB с запасом. Тернарные веса не требуют dotprod-пути: Q4_K на этом дереве тоже идёт без dotprod, так что «штраф за отсутствие dotprod» для TQ не уникален.

Минусы: `nrows=1` и отсутствие repack бьют по prefill; распаковка троичных кодов — это ALU-нагрузка на каждый байт; качество 1.58-bit ниже Q4 при том же размере; нагрев при 4×A75.

Оценки (гипотезы, не замеры; опорная точка — факт из README: Qwen2.5-0.5B Q4_K_M = 25.2 tok/s decode, 48 tok/s prefill, файл 0.40 GB):

| Модель | Файл | Трафик/токен | Decode (гипотеза) | Prefill (гипотеза) |
|---|---:|---:|---:|---:|
| Qwen2.5-0.5B Q4_K_M | 0.40 GB | 0.40 GB | **25 (факт)** | **48 (факт)** |
| Bonsai 1.7B TQ2_0 | 0.62 GB | 0.62 GB | 13–18 | 12–18 |
| BitNet 2B4T TQ1_0 (F16 embed) | 1.10 GB | ~1.1 GB | 9–12 | 10–14 |
| BitNet 2B4T TQ1_0 (Q4_K embed) | ~0.63 GB | ~0.63 GB | 13–16 | 10–14 |
| Bonsai 2-2B Q2_0 (hybrid attn) | 0.55 GB | ~0.55 GB | 15–20 | 12–18 |
| Qwen3-4B Q4_K_M | ~2.5 GB | 2.5 GB | 4–6 | 5–8 |

Вывод: смысл TQ на 845 — **качество 1.5–2B при скорости 0.5B**, а не абсолютная скорость. Для «длинный system prompt + RAG» тернарник плох: 1900-токенный `bench/prompt.txt` при 14 tok/s prefill — это ~135 с TTFT. Thermal: 4×A75 на 2.8 GHz держат burst; через 1–2 минуты ожидаю падение на 20–35% (гипотеза, мерить).

## Кандидаты (проверено по HF API и заголовкам GGUF)

| Модель / репозиторий | Файл, размер | Лицензия | Формат | Статус |
|---|---|---:|---|---|
| `ewchampion/Ternary-Bonsai-1.7B-TQ2_0-GGUF` | 624.6 MB | Apache-2.0 | qwen3, `file_type=37`, 196×TQ2_0 + Q6_K embed + F32 | **Загрузится, лучший bring-up** |
| `ihtesham0345/bitnet-b1.58-2B-4T-llamakind` | 1.106 GB | не указана (база MIT) | arch `bitnet`, `file_type=36`, 210×TQ1_0 + F16 embed | Загрузится, но нужен патч relu2 и желательно Q4_K embed |
| `prism-ml/Ternary-Bonsai-1.7B-gguf` | F16 3.44 GB; Q2_0_g64 490 MB; PQ2_0 463 MB | Apache-2.0 | qwen3, Q2_0 (42) / PQ2_0 (кастом) | Q2_0_g64 — рабочий вариант «не-TQ»; PQ2_0 чужие ядра |
| `prism-ml/Ternary-Bonsai-2-2B-gguf` | PQ2_0 517 MB, Q2_0 547 MB, PTQ1_0 429 MB | Apache-2.0 | qwen35 + `prism.hadamard` v2, 1.88B | Q2_0 потенциально грузится (есть qwen35+hadamard+Q2_0), PTQ1_0/PQ2_0 — нет |
| `prism-ml/Ternary-Bonsai-2-4B-gguf` | 0.95/1.15/1.21 GB | Apache-2.0 | qwen35, 4.21B, hadamard | Аналогично; на 845 будет ~8–10 tok/s |
| `microsoft/bitnet-b1.58-2B-4T-gguf` | 1.19 GB | MIT | arch `bitnet-b1.58`, тип 36 | **Не грузится** (arch + тип) |
| `tiiuae/Falcon3-1B-Instruct-1.58bit-GGUF` | 1.36 GB | Falcon3 license (other) | arch `llama`, 126×тип 36, F16 embed | **Не грузится**; TQ-вариантов нет |
| `xxxn3m3s1sxxx/BitNet-2B4T-b1.58-ATLAS` | 1.12 GB | MIT | `.atlas`, не GGUF | Неприменимо |

Отдельно: `microsoft/bitnet-b1.58-2B-4T` (safetensors) — источник для конвертации, но родного конвертера в дереве нет; `tdh111/...` — IQ2_Bn для ik_llama.cpp, не наш тип.

## План интеграции

**Шаг 0. Патч движка (private fork, 5 строк).** `src/models/bitnet.cpp`: читать `bitnet.hidden_activation` через `LLM_KV_HIDDEN_ACT` и по умолчанию ставить `LLM_FFN_RELU_SQR` для arch `bitnet`; в `llama-model.cpp:1122` добавить `{"relu2", LLM_FFN_RELU_SQR}`; в графе использовать `hparams.llm_ffn_op` вместо константы. Без этого BitNet-кандидат бессмысленен.

**Шаг 1. Скачать 2 файла (1.7 GB суммарно, не больше).**
```powershell
curl -L -o models\ternary\Ternary-Bonsai-1.7B-TQ2_0.gguf https://huggingface.co/ewchampion/Ternary-Bonsai-1.7B-TQ2_0-GGUF/resolve/main/Ternary-Bonsai-1.7B-TQ2_0.gguf
curl -L -o models\ternary\bitnet-b1.58-2B-4T-tq1_0.gguf https://huggingface.co/ihtesham0345/bitnet-b1.58-2B-4T-llamakind/resolve/main/bitnet-b1.58-2B-4T-tq1_0.gguf
```
**Шаг 2. `scripts/gguf-scan.py`** — читает заголовок, печатает arch, `file_type`, гистограмму типов, падает, если нет 34/35 или arch не поддержан. Дешёвая pre-flight проверка вместо «загрузил 1 ГБ и получил unknown type».

**Шаг 3. Desktop CLI.** Собрать `build/llama-clover` на ПК, прогнать `llama-cli -m ... -t 4 -c 2048 -n 64 --temp 0.7 -p "..."` и, по возможности, `llama-perplexity -f wiki.txt -c 512`, чтобы поймать мусор от relu2/квантования до телефона.

**Шаг 4. Device CLI (не сейчас, устройство занято).**
```powershell
.\scripts\bench-cli.ps1 -Model models\ternary\Ternary-Bonsai-1.7B-TQ2_0.gguf -Threads 4 -Prompt 512 -Predict 128 -Extra "-ngl 0 -c 2048 -fa 1"
```
Отдельно снять sustained 10 минут и thermal status.

**Шаг 5. JNI.** Загрузка уже работает без изменений. В `app/src/main/cpp/snapcore.cpp` после `llama_model_load_from_file` добавить `llama_model_ftype()` (`include/llama.h:648`) и `llama_model_arch()`/`model_desc`, вернуть в `nativeInfo` поля `ftype`, `quant`, `is_ternary`. Это надёжнее, чем имя файла.

**Шаг 6. UI/детект.** Сейчас `ModelStore.kt:69 quantFromName()` парсит только имя, а `GgufMeta.kt` читает метаданные без таблицы тензоров. Добавить в `GgufMeta` чтение tensor infos (после tokenizer-массивов) и счётчик типов 34/35/42; в `ModelCardItem` (`Panels.kt:156`) — бейдж `TERNARY 1.58-bit` и предупреждение «качество ниже Q4; длинный prefill; не переквантовывать обычные модели». Пресет Tuning: threads 4, ctx 2048, kv q8_0, mmap on, gpuLayers 0, batch 64.

**Шаг 7. Запрет on-the-fly.** В приложении квантайзера нет — и не добавлять на устройстве. Если появится: разрешать TQ только при `arch==bitnet` или уже тернарном источнике; F16-источник 1.7B (3.4 GB) на 6 GB всё равно не влезет, плюс наш `quantize_tq*` игнорирует imatrix.

**Шаг 8. Quality gate.** 22 промпта из `bench/quality-prompts.json` против Qwen2.5-0.5B (baseline) и, если влезет, Qwen3-1.7B Q4_K_M. Отдельно проверить chat template: у community-файла BitNet в метаданных упрощённый шаблон (`User: ...`), а нужен Llama-3 header-формат; `snapcore.cpp:328` берёт шаблон из GGUF, так что шаблон надо переопределить в приложении или пропатчить GGUF.

## Риски

1. **BitNet без патча relu2** — «связный, но неправильный» текст; замеры качества недействительны.
2. **TQ из обычной модели** — деградация до мусора (нет group-scale, нет imatrix); это главный запрет для UI.
3. **Prefill** — 2k промпта = 2–3 минуты; для чата держать короткий system prompt.
4. **Community-квант Bonsai TQ2_0** — аплоадер проверил только связность, eval нет; возможна потеря относительно Q2_0/PQ2_0.
5. **qwen35 + hadamard + Q2_0** (2B/4B Bonsai) — путь есть, но на CPU overhead hadamard и корректность надо мерить против F16.
6. **Лицензии** — у community-конверсий лицензия не указана; для поставки брать `prism-ml`/`microsoft` (Apache-2.0/MIT).
7. **Thermal/троттлинг** — все цифры выше только для холодного устройства.

## Факт / гипотеза / надо измерить

**Факт:** типы, bpw, ARM-ядра с `vmlal_s8`, `q8_K`-активации, отсутствие repack и `nrows=1`; imatrix игнорируется; официальный MS GGUF и Falcon3 GGUF несовместимы (arch/тип 36); BitNet-граф использует SiLU при обучении с relu2 (PR #26751); Vulkan TQ есть, OpenCL нет; APK CPU-only; размеры/типы/лицензии файлов из таблицы; Qwen2.5-0.5B = 25.2/48 tok/s.

**Гипотеза:** 13–18 tok/s для 1.7B TQ2_0; 9–16 для BitNet в зависимости от embed; падение 20–35% на sustained; TQ1_0 медленнее TQ2_0 на вес из-за base-3 распаковки.

**Надо измерить:** decode/prefill/TTFT на 845 для 1.7B TQ2_0 и BitNet TQ1_0; peak PSS и KV при 2k/4k; влияние Q4_K vs F16 embedding на lm_head; качество 22-промптного gate; корректность qwen35+hadamard Q2_0 (Bonsai 2B) против F16; thermal через 10 минут.
