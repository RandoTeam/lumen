# 13 — Clover на Snapdragon: что пришлось сделать

## Коротко

- Clover **загружается и отвечает** на MI 8 (SD845, 6 GB) через наше приложение.
- Скорость на 2026-11-07: **prefill ~2.4 tok/s, decode ~0.6 tok/s**, TTFT ~26 s
  на 64-токенном промпте. Это storage-bound demo, не «скорость».
- Для этого движок собран не из mainline llama.cpp, а из PR **#31306**, коммит
  **4838ffb0c8ade92f34d9157deea38d92758ab824** (ветка `clover-pr`).
- Mainline llama.cpp (f7591a6) содержит `clover_moe`, но его autoregressive-граф
  **не использует** block-diffusion тензоры и выдаёт бессвязный текст на нашем файле.

## Почему не mainline

Наш GGUF: `Clover-35B-A3B-Preview.APEX-I-NanoPlus-V2.1.gguf`, 853 тензора.

При загрузке mainline:

```
key not found in model: clover_moe.expert_shared_count
```

Добавили дефолт `n_expert_shared = 1` — загрузилось, но:

```
wrong number of tensors; expected 853, got 733
```

Разница ровно **120 тензоров** = 30 recurrent-слоёв × 4:

| Тензор | Форма | Назначение |
| --- | --- | --- |
| `blk.N.ssm_conv1d.weight_right` | `[3, 8192]` | правая часть conv для block diffusion |
| `blk.N.ssm_bd_history.weight` | `[32]` | история block diffusion |
| `blk.N.ssm_bd_history.bias` | `[32]` | то же |
| `blk.N.ssm_bd_logit_scale.weight` | `[32]` | масштаб логитов block diffusion |

Коммит `4838ffb` — это версия PR, которая **читает и использует** эти тензоры.
Коммит `ed26f2f` (голова PR, 2026-11-01) заменил граф на более простой AR-вариант
и удалил их; он загружает файл только если игнорировать 120 тензоров, и качество
на нашем файле плохое (проверено: бессвязный текст).

Поэтому:

```
external/llama-clover-src   = worktree на 4838ffb (используется приложением)
external/llama.cpp          = mainline + эксперименты (не используется APK)
```

`app/src/main/cpp/CMakeLists.txt` указывает `LLAMA_SRC` на `external/llama-clover-src`.

## Как воспроизвести

```powershell
cd external\llama.cpp
git fetch --depth 1 origin 4838ffb0c8ade92f34d9157deea38d92758ab824
git worktree add ..\llama-clover-src 4838ffb0c8ade92f34d9157deea38d92758ab824
```

Сборка CLI под Android:

```powershell
$env:ANDROID_NDK="C:\Users\<user>\AppData\Local\Android\Sdk\ndk\28.2.13676358"
cmake -S external\llama-clover-src -B build\llama-clover-android -G Ninja `
  -DCMAKE_TOOLCHAIN_FILE="$env:ANDROID_NDK\build\cmake\android.toolchain.cmake" `
  -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-28 `
  -DGGML_NATIVE=OFF -DGGML_OPENMP=OFF -DGGML_LLAMAFILE=OFF `
  -DGGML_CPU_KLEIDIAI=OFF -DLLAMA_OPENSSL=OFF -DLLAMA_CURL=OFF `
  -DLLAMA_BUILD_SERVER=OFF -DLLAMA_BUILD_TESTS=OFF `
  -DGGML_BACKEND_DL=OFF -DGGML_CPU_ALL_VARIANTS=OFF
cmake --build build\llama-clover-android -j 8 --target llama-completion
```

Запуск (на устройстве):

```sh
cd /data/local/tmp/sllm
LD_LIBRARY_PATH=. ./llama-completion \
  -m Clover-35B-A3B-Preview.APEX-I-NanoPlus-V2.1.gguf \
  -c 1024 -t 4 -n 20 -p "Explain in one sentence why the sky is blue." -cnv -st
```

Проверенный ответ (20 токенов, обрезан лимитом):

```
<think>
We need answer concise one sentence. Need explain sky blue: Rayleigh scattering of sunlight by
```

## Замеры

Условия: MI 8, Android 15, CPU-only, 4 потока, ctx 1024/4096, KV q8_0,
`mmap=true`, экран включён, без зарядки.

| Что | Значение | Источник |
| --- | ---: | --- |
| Загрузка (CLI, холодная) | 24.2–27.1 s | `common_perf_print` |
| Prompt eval | 0.90–0.98 tok/s (CLI, 19 tok) | CLI |
| Decode | 0.29–0.38 tok/s (CLI) | CLI |
| Prompt eval в приложении | **2.34–2.44 tok/s** (64 tok, 26–27 s) | `session.jsonl` |
| Decode в приложении | **0.60–0.67 tok/s** (не изменился после persistent threadpool) | `session.jsonl` |
| TTFT | 26.2–27.3 s | `session.jsonl` |
| Генерация | 36 токенов за 54 s после prefill (0.67 tok/s) | `session.jsonl` |
| RSS процесса | 2.81 GB | `/proc/self/status` |
| Peak RSS | 4.26 GB | `/proc/self/status` |
| PSS | ~2.98–3.97 GB | `dumpsys meminfo` |
| Compute buffer | 127.5 MiB | llama.cpp log |
| Recurrent state | 62.8 MiB | llama.cpp log |

Приложение при этом **не убито LMK** и продолжает отвечать.

### Почему так медленно

- 8 из 256 экспертов × 40 слоёв × ~1.1 MB = **~350 MB весов на токен**.
- 0.6 tok/s → порядка **200 MB/s** эффективного чтения (UFS + page cache).
- Persistent threadpool (см. `14-BACKENDS.md`) поднял Qwen с 18 до 25 tok/s,
  но Clover не изменился: предел — не CPU, а чтение экспертов.
- В 6 GB влезает ~2.5–3 GB page cache; это несколько тысяч экспертов из 10 240.
- Без per-layer квот и prefetch горячие эксперты вытесняются холодными.
- CPU без dotprod: считающая часть тоже не бесплатна, но главный предел — I/O.

## Что дальше (по приоритету)

1. **Экспертный prefetch**: пока считается слой L, читать вероятных экспертов L+1
   через `madvise(MADV_WILLNEED)` или фоновый `pread` в пул.
2. **Per-layer квоты** и LRU/aging по счётчикам; профиль `expert-profile.bin`
   как начальное приближение.
3. **Батчинг чтений** нескольких экспертов одного слоя одним потоком.
4. **mmap vs pread** A/B на UFS 2.1; сейчас только mmap.
5. **KV q4_0** и контекст 2K для Clover (меньше памяти под KV, больше под page cache).
6. **GPU-кэш экспертов** через Vulkan + upstream `moe_cache_size` — после CPU-базы.
7. Quality gate: 22 промпта, 2 повтора, при `max_tokens` ≥ 1200 (модель пишет
   `<think>` и без запаса обрывается).

## Честная формулировка

> Clover 35B-A3B запускается на Snapdragon 845 в 6 GB и генерирует осмысленный
> текст. Скорость ~0.6 tok/s на decode и ~2.4 tok/s на prefill: это демонстрация
> работоспособности и полигон для экспертного кэша, а не повседневный чат.
> Числа получены на устройстве, одного прогона; quality gate ещё не пройден.
