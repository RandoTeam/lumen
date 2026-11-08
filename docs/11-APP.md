# 11 — Приложение SnapdragonLLM (v0.1)

Статус: **собрано и запущено на MI 8 (SD845, Android 15)** 2026-11-07.
Это первый рабочий чат; движок уже отдаёт честные метрики и логи.

## Что это

Один APK, внутри — нативное C++-ядро на llama.cpp. Модели лежат вне APK.
Приложение не требует аккаунта, сети и не отправляет текст наружу.

```
┌─────────────────────────────── Compose UI ───────────────────────────────┐
│  Chat · Models · Tuning · Telemetry · Logs                               │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │ AppController (state, coroutines)
┌──────────────────────────────────▼───────────────────────────────────────┐
│  Engine.kt  (JNI)                                                        │
│  load / resetChat / addMessage / begin / next / stats / trim / free      │
└──────────────────────────────────┬───────────────────────────────────────┘
┌──────────────────────────────────▼───────────────────────────────────────┐
│  libsnapcore.so  (static llama.cpp + common + ggml CPU)                   │
│  GGUF · chat template · prefill · sampler · KV · JSONL counters          │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │ mmap
                         GGUF на UFS / page cache
```

## Экраны

| Экран | Что делает |
| --- | --- |
| **Chat** | потоковый чат, стоп, new chat, статистика под ответом |
| **Models** | скан `.gguf`, импорт через SAF, карточки модели, выбор |
| **Tuning** | threads, ctx, batch, KV, mmap/mlock, flash attention, MoE cache, sampling, system prompt |
| **Telemetry** | tok/s, TTFT, prefill, KV, RSS/peak, RAM, thermal, батарея, конфиг, SoC |
| **Logs** | append-only `session.jsonl`, share/clear |

Дизайн: тёмный фон `#07080A`, акцент лайм `#D9FF57`, мятный `#6BE8C6`,
тонкие границы, крупная типографика, monospace для чисел. Без чужих ассетов.

## Память

Цель: **50–100 MB на оболочку приложения** без учёта страниц модели.
Модель учитывается отдельно: `mmap` добавляет file-backed страницы в PSS,
но они вытесняемы и не являются private heap.

Что уже сделано:

- один процесс, без Flutter, без Hilt/Room/Navigation;
- только Compose + core-ktx + activity + lifecycle;
- нативное ядро статически в одном `.so`, без динамических бэкендов;
- `n_batch` по умолчанию 128 (меньше compute-буфер, чем 256);
- `onTrimMemory` → `nativeTrim`: на MODERATE чистит KV, на COMPLETE — KV+sampler+историю;
- `Debug`/`Release` R8 (release) для удаления неиспользуемого кода;
- телеметрия показывает `VmRSS`, `VmHWM`, `VmSize` и `availMem`.

Измерено (release APK, R8, 2026-11-07):

| Состояние | PSS | Комментарий |
| --- | ---: | --- |
| Оболочка без модели | **60.0 MB** | Java 5.5, native 7.2, code 11.7, graphics 20.5 |
| Qwen2.5-0.5B, ctx 4096, batch 128 | ~531 MB | из них ~380 MB — страницы mmap модели |
| Clover 35B-A3B, ctx 4096 | 2.98–3.97 GB | peak RSS 4.26 GB, LMK не убил |

Цель 50–100 MB для оболочки **достигнута**: 60 MB без модели.

Скорость (release, 4 потока, persistent threadpool):

| Модель | TTFT | prefill | decode |
| --- | ---: | ---: | ---: |
| Qwen2.5-0.5B Q4_K_M | 1.3 s | 48 tok/s | **25.2 tok/s** |
| Clover 35B-A3B | 26.2 s | 2.34 tok/s | 0.6 tok/s |

Compute buffer при batch=128 — **127.5 MiB**; при 256 был 154.8 MiB.
Подробный A/B бэкендов: `docs/14-BACKENDS.md`.

## Конфиг без переустановки

Приоритет: `defaults < SharedPreferences < config.json < Tuning в приложении`.

`config.json` читается при старте из:

```
/sdcard/Android/data/dev.snapdragonllm/files/config.json
```

```json
{
  "threads": 4,
  "ctx": 4096,
  "batch": 128,
  "kv": "q8_0",
  "mmap": true,
  "mlock": false,
  "flash_attn": true,
  "gpu_layers": 0,
  "moe_cache_mb": 0,
  "max_tokens": 512,
  "temp": 0.7,
  "top_p": 0.95,
  "top_k": 40,
  "system_prompt": "..."
}
```

`Apply` в Tuning перезагружает движок и **сохраняет чат**: история заново
добавляется в шаблон, KV пересчитывается при следующем запросе.

## Логи

`/sdcard/Android/data/dev.snapdragonllm/files/logs/session.jsonl`

Одна строка — один запрос или загрузка модели:

```json
{"type":"request","ts":"...","run_id":"...","device":{...},"model":{...},
 "config":{...},"request":{...},
 "metrics":{"ttft_ms":...,"prefill_tok_s":...,"decode_tok_s":...,"kv_cache_mb":...},
 "memory":{"rss_mb":...,"peak_rss_mb":...,"vm_mb":...}}
```

Тексты промптов по умолчанию **не** пишутся (только длины). Логи доступны
через `adb pull` и кнопку Share.

## Dev-loop с телефоном

```powershell
# 1. Собрать и поставить
.\scripts\build-apk.ps1 -Install

# 2. Запушить конфиг и модель, запустить
.\scripts\deploy.ps1 -PushConfig -Model .\models\tiny\Qwen2.5-0.5B-Instruct-Q4_K_M.gguf

# 3. Смотреть нативные логи
adb logcat -s SLLM:*

# 4. Снять логи приложения
adb pull /sdcard/Android/data/dev.snapdragonllm/files/logs/session.jsonl
```

`adb root` на этой прошивке работает (userdebug), отдельный `su` не нужен.
Приложение остаётся в песочнице; для debug APK доступен `run-as`.

## Ограничения v0.1

- Только CPU backend. Vulkan/OpenCL — следующие шаги (Adreno 630 видит оба API).
- Полный prompt пересобирается на каждый запрос (просто и надёжно; KV-реюз — позже).
- Нет tool-calling, изображений, аудио. Сначала текстовый фундамент.
- Clover 35B-A3B: загружается через патч llama.cpp; block-diffusion тензоры
  читаются, но в autoregressive-графе не используются (см. `13-CLOVER.md`).
- `moe_cache_mb` — upstream device-cache; на CPU **не бесполезен, а опасен**:
  `llama_moe_cache` требует GPU и бросает `MoE cache requires a GPU backend`.
  Держать 0 до рабочего Vulkan/OpenCL.

## Следующее

1. Release APK + R8, замерить PSS оболочки без модели.
2. A/B `batch` 64/128/256, `threads` 2/4/6, `kv` f16/q8_0/q4_0.
3. Clover: TTFT/decode, hit rate, SSD MB/token, thermal 10 минут.
4. Vulkan backend + MoE cache на Adreno 630.
5. Экспертный prefetch: `madvise(WILLNEED)` по профилю и online-счётчикам.
