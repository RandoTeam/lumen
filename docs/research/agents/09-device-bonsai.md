# Отчёт 09. Device-тест Ternary-Bonsai-2-4B + LoRA на MI 8

Источник: параллельное исследование SnapdragonLLM, 2026-11-08.
Устройство: MI 8, SD845, 6 GB, Android 15, CLI из 4838ffb.

## TL;DR

- **База загрузилась.** `Ternary-Bonsai-2-4B-Q2_0.gguf` (1.21 ГБ) открывается на `4838ffb` без патчей: arch `qwen35`, 426 тензоров, только `Q2_0` + `F32`.
- **Скорость низкая:** prefill **3.1–3.7 tok/s**, decode **2.3–2.6 tok/s** на 4×A75. Загрузка ~2.15 с, RSS ~1.32 ГиБ.
- **LoRA работает технически:** `--lora`/`--lora-scaled` есть, адаптер 2.4 МБ грузится, при `scale 1` детерминированный вывод расходится с base, при `scale 100` ломается в «here here here». Но адаптер сделан под **PTQ1_0**, а PTQ1_0 этот бинарь **не открывает** (`invalid ggml type 143`).
- **PTQ1_0/PQ2_0 не поддерживаются** сборкой 4838ffb: нужен PrismML-fork. Q2_0 — единственный файл, который реально едет на upstream-подобном бинаре.
- Приложение возвращено в работу, файлы лежат в `/data/local/tmp/ternary/`.

## Файлы и метаданные

Дерево HF получено через `https://huggingface.co/api/models/.../tree/main`.

| Файл | Размер (API/факт) | Что это | Статус на 4838ffb |
|---|---:|---|---|
| `Ternary-Bonsai-2-4B-Q2_0.gguf` | 1 212 103 520 B (1.13 GiB) | языковая модель, Q2_0 | **грузится** |
| `Ternary-Bonsai-2-4B-PTQ1_0.gguf` | 949 549 920 B | dense trits, тип 143 | **не грузится** |
| `Ternary-Bonsai-2-4B-PQ2_0.gguf` | 1 146 465 120 B | 2-bit slots | не качал, по README/исходникам тоже нет |
| `Ternary-Bonsai-2-4B-PQ2_0-MTP-Q8_0.gguf` | 1 274 642 176 B | PQ2_0 + MTP head | не качал, нужен fork |
| `Ternary-Bonsai-2-4B-DFlash2-BF16.gguf` | 1 575 278 208 B | block drafter | не нужен для чата |
| `Ternary-Bonsai-2-4B-mmproj-Q8_0.gguf` | 366 894 624 B | vision tower | не проверял |
| `Ternary-Bonsai-2-4B-Uncensored-LoRA.gguf` | 2 368 992 B | GGUF LoRA, rank-1 | **применяется** |

Точные URL:

```
https://huggingface.co/prism-ml/Ternary-Bonsai-2-4B-gguf/resolve/main/Ternary-Bonsai-2-4B-Q2_0.gguf
https://huggingface.co/prism-ml/Ternary-Bonsai-2-4B-gguf/resolve/main/Ternary-Bonsai-2-4B-PTQ1_0.gguf
https://huggingface.co/AtomicChat/Ternary-Bonsai-2-4B-Uncensored-LoRA-GGUF/resolve/main/Ternary-Bonsai-2-4B-Uncensored-LoRA.gguf
```

SHA256 сверены с LFS oid из API (и на хосте, и на телефоне):

```
Q2_0 : 64a5183b44921bdb5ac4a6f5f92007821d6ebe5c66997d3c147fd214ab9433d4
PTQ1_0: 56711263ec9a9ffeaa74a3a0e58dece8127cf9268404746c721113ca76994d60
LoRA : 605e9dfba951a2ddd86b4975fcb2d064cebb6a284ebec396d5b93d58710138a9
```

Заголовок Q2_0 (собственный парсер `models/ternary/gguf_inspect.py`):

```
GGUF version 3  tensors 426  kv 50
general.architecture = "qwen35"
general.name = "Ternary-Bonsai-2-4B"   general.size_label = "4B"
general.file_type = 41
qwen35.block_count = 32   embedding_length = 2560   feed_forward_length = 9216
attention.head_count = 16 / head_count_kv = 4   context_length = 262144
full_attention_interval = 4   ssm.state_size = 128   ssm.inner_size = 4096
prism.hadamard.version = 2   block_size = 512   tied_output = true
prism.hadamard.sign_widths = [2560, 4096, 9216]   gdn_v_grouped = true
tokenizer.ggml.tokens = 248320   chat_template = 7756 chars
TENSOR TYPES: {'F32': 225, 'Q2_0': 201}
```

«2.4B» — маркетинговое имя: `llama-bench` показывает **4.21 B** параметров. Chat template — Qwen-подобный (`<|im_start|>`, `<think>`), thinking включён по умолчанию.

LoRA:

```
general.architecture = "qwen35"   general.type = "adapter"   adapter.type = "lora"
adapter.lora_alpha = 1.0
prism.abliterate.base_gguf = "Ternary-Bonsai-2-4B-PTQ1_0.gguf"
prism.abliterate.base_sha256 = "56711263..."
prism.abliterate.sites = "64"
TENSOR TYPES: {'F32': 128}
```

128 тензоров = 64 пары `lora_a/lora_b` (rank 1) на `ffn_down`, `ssm_out`, `attn_output`. Формат — **родной GGUF LoRA**, не safetensors/PEFT, конвертация не нужна.

## Что запускал

```powershell
adb shell am force-stop dev.snapdragonllm
# MemAvailable: 3 582 004 kB, SwapFree: 1 584 120 kB
adb push ...Q2_0.gguf /data/local/tmp/ternary/base.gguf      # 39.2 MB/s
adb push ...LoRA.gguf /data/local/tmp/ternary/lora.gguf
adb push build\llama-clover-android\bin\libllama-bench-impl.so /data/local/tmp/sllm\
adb shell chmod 755 /data/local/tmp/sllm/llama-bench
```

`llama-bench` на телефоне был `rw-rw-rw-` и без `libllama-bench-impl.so` — без этих двух шагов не запускался. Версия бинаря:

```
version: 0.7.0-dev (build 1, commit 4838ffb)
built with Clang 19.0.1 for Android aarch64
```

Основной прогон (prompt через файл, иначе adb съедает кавычки):

```sh
cd /data/local/tmp/sllm
LD_LIBRARY_PATH=. ./llama-completion -m /data/local/tmp/ternary/base.gguf \
  -c 2048 -t 4 -n 64 -f /data/local/tmp/ternary/prompt.txt -no-cnv --no-warmup
```

`-h` подтверждает:

```
--lora FNAME
--lora-scaled FNAME:SCALE,...
--warmup, --no-warmup
-cnv, --conversation, -no-cnv, --no-conversation
```

## Ключевые сырые строки

Base, `n=64`:

```
load time =    2145.99 ms
prompt eval time = 2145.47 ms / 8 tokens (268.18 ms per token, 3.73 tokens per second)
eval time = 24726.98 ms / 63 runs (392.49 ms per token, 2.55 tokens per second)
total time = 26914.81 ms / 71 tokens
graphs reused = 62
```

Пик памяти (сэмплирование `/proc/<pid>/status` каждые 2 с):

```
rss_kb=1386660 hwm_kb=1386704 avail_kb=3646096 cpu3gold=81400 batt=310
```

`llama-bench`, чисто, после прогрева (тело SoC уже горячее):

```
| qwen35 4B Q2_0 | 1.12 GiB | 4.21 B | CPU | 4 | pp128 | 3.08 ± 0.00 |
| qwen35 4B Q2_0 | 1.12 GiB | 4.21 B | CPU | 4 |  tg32 | 2.32 ± 0.12 |
build: 4838ffb (1)
```

PTQ1_0 — точная ошибка:

```
E gguf_init_from_reader: tensor 'token_embd.weight' has invalid ggml type 143. should be in [0, 43)
E llama_model_load: error loading model: llama_model_loader: failed to load model from .../ptq1.gguf
E llama_completion: error: unable to create context
```

LoRA, `--lora-scaled lora.gguf:1`, greedy `--temp 0 -s 42`, 96 токенов:

```
base : "...  \"Hello! I will reply in one short sentence.\" (A bit long) ..."
+lora: "...  \"Hello! How can I help you?\" (Good, but maybe too long?) ..."
SHA256 сгенерированной части: 0F56AB4D... vs 9945381B...
```

При `scale=100`:

```
Hello! Reply in one short sentence. here here here here here here here here ...
```

При `scale=0` вывод побайтово совпал с base. При неверном пути к адаптеру — явная ошибка `failed to open ... No such file or directory`. Значит LoRA реально подхватывается и влияет на граф.

Аномалия: с LoRA поля таймингов в `common_perf_print` обнуляются (`load time = 0.43 ms`, `prompt eval time = 0.00 ms`), а реальное время уходит в `unaccounted time = 2183.65 ms`. Это баг/особенность ветки, не «мгновенная загрузка».

## Метрики

| Метрика | Base | Base + LoRA (scale 1) |
|---|---:|---:|
| Load time | 2145.99 ms | ~2.2 с (в `unaccounted`, счётчик сломан) |
| Prefill (8 tok, c=2048) | 3.73 tok/s | не печатается; ~те же 2.2 с |
| TTFT | ~2.15 с | ~2.2 с |
| Decode | 2.55 tok/s (392 ms/tok) | 2.41 tok/s (416 ms/tok) |
| Bench pp128 | 3.08 tok/s | не мерил |
| Bench tg32 | 2.32 tok/s | не мерил |
| RSS (плато) | 1 386 660 kB | 1 390 904 kB (+4.2 MB) |
| VmHWM | 1 386 704 kB | 1 390 948 kB |
| MemAvailable до/во время | 3 856 220 → 3 646 000 kB | 3 896 000 → 3 681 000 kB |
| MemFree (минимум) | ~63 000 kB | ~73 000 kB |
| Thermal cpu3-gold | 34.8 → **90.6 °C**, откат к 80–82 | 43.6 → 83.1 °C |
| Battery temp | 30.0 → 31.0 °C | 32.0 → 33.0 °C |
| Battery level | 71% (в конце) | — |

Память в норме: 1.32 ГиБ RSS при 5.77 ГБ RAM, swap использован примерно на 0.7 ГБ ещё до теста (SwapFree 1.58 из 2.29 ГБ). Контекст 2048 при таком размере модели влезает; 262k, разумеется, нет.

`dumpsys meminfo` во время прогона показывает `Free RAM ~2.42 GB`, `Used RAM ~3.48 GB`, `Lost RAM 0.37 GB`; CLI-процесс под root в per-process списке не виден, поэтому RSS брал из `/proc`.

Обратите внимание: 4 потока на 4×A75 упираются в троттлинг за ~20–30 секунд. Первый прогон до прогрева дал 3.22/2.41, читый повторный — 3.08/2.32; разница в пределах теплового дрейфа, не «ускорение».

## LoRA: что именно проверено

- Формат **GGUF LoRA**, не PEFT/safetensors. `general.type=adapter`, `adapter.type=lora`, alpha 1.0, 64 rank-1 сайта. Конвертировать нечего.
- `--lora` и `--lora-scaled` поддерживаются, ошибок загрузки нет.
- Эффект доказан тремя способами: `scale=0` ≡ base, `scale=1` меняет greedy-продолжение на ~85-м токене, `scale=100` разрушает вывод. Плюс неверный путь даёт явную ошибку.
- **Но** адаптер обучен на `PTQ1_0` (sha прописан в метаданных). На `Q2_0` он применяется механически (дельта считается поверх деквантованных весов), однако авторских замеров на Q2_0 нет. Утверждать, что «refusal removal» работает так же, нельзя.
- Оверхед: +4.2 МБ RSS, скорость в пределах шума.

## Что мешает и какой нужен патч

1. **PTQ1_0/PQ2_0 не читаются вообще.** `ggml.h` в 4838ffb знает `TQ2_0=35` и `Q2_0=42`, но не типы 143/144. Минимальный патч — не «одна строка»: нужно добавить `GGML_TYPE_PTQ1_0`/`PQ2_0` в `ggml_type_traits`, деквант/`vec_dot` ядра (тернарные с group-wise FP16 scale + Hadamard-базис), маппинг `general.file_type` и разрешить их в загрузчике qwen35. Практически это и есть патчи PrismML-форка. Пересобирать в этом тесте было запрещено — не делал.
2. **Для Q2_0 патч не нужен.** Модель грузится и генерирует.
3. **Скорость.** 2.3–2.6 tok/s — это в 10 раз медленнее Qwen2.5-0.5B Q4_K_M (26.6 tok/s) на том же железе. Причина: 4.21B параметров + Hadamard-преобразование активаций на каждый матмул, без репака. Для интерактива на SD845 это непрактично; для батчевых коротких задач — терпимо.
4. **Thinking по умолчанию.** При `-no-cnv` и сыром промпте модель всё равно уходит в `<think>` и съедает весь `-n` на рассуждения. Для приложения нужен chat template с `enable_thinking=false` (в шаблоне он есть) либо явная обрезка.
5. **Сломанные тайминги при LoRA.** `load time`/`prompt eval` печатаются как 0; для оценки TTFT приходится смотреть `unaccounted`.
6. **LoRA-база не та.** Если нужен «как задумано» — нужен PTQ1_0 + PrismML fork, а не Q2_0 на 4838ffb.

## Честно: что не проверено

- **Vision/mmproj** — не качал и не запускал.
- **MTP и DFlash2** — не проверял, требуют fork и отдельных флагов.
- **PQ2_0 runtime-ошибка** — не воспроизводил, вывод сделан по README и по отсутствию типа в `ggml.h`; PTQ1_0-ошибка воспроизведена реально.
- **Refusal-метрика LoRA** — не мерил (нет публичного набора под рукой); проверен только факт применения и расхождения выводов.
- **Длинный контекст (8k/32k/262k)** — не тестировал, только 1024/2048.
- **`llama-bench` с LoRA** — не запускал.
- **Интеграция в APK** — не трогал; приложение запущено на прежней сборке.

## Состояние после теста

- Приложение: `am start -n dev.snapdragonllm/.MainActivity` выполнено, процесс `dev.snapdragonllm` жив.
- На хосте: `C:\OpenCode\SnapdragonLLM\models\ternary\` — `Ternary-Bonsai-2-4B-Q2_0.gguf`, `Ternary-Bonsai-2-4B-PTQ1_0.gguf`, `Ternary-Bonsai-2-4B-Uncensored-LoRA.gguf`, `gguf_inspect.py`, `run_model.sh`, `prompt.txt`, логи и `*_header.txt`. Скачано 2.16 ГБ — в лимите 3 ГБ.
- На телефоне: `/data/local/tmp/ternary/` — `base.gguf` (Q2_0), `lora.gguf`, `ptq1.gguf`, `prompt.txt`, `run_model.sh` и все логи прогонов. Ничего не удалено.
- В `/data/local/tmp/sllm/` добавлен только `libllama-bench-impl.so` (его не хватало) и выставлен `chmod 755` на `llama-bench`; модели и app-каталог не тронуты.
