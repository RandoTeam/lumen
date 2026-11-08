# 05 — Логирование и настройка на ходу

## Цель

Любой переключатель (backend, потоки, GPU-слои, слоты кэша, KV, prefill chunk,
спекуляция) меняется **без переустановки**; каждое изменение оставляет след в логе; любой
замер можно повторить по записи.

## Три вида логов

| Лог | Что пишет | Где |
| --- | --- | --- |
| `session.jsonl` | один запрос: конфиг, метрики, ошибки | app files |
| `runs.jsonl` | одна серия замеров (benchmark) | app files |
| `crash.log` | нативный стек, память, последний конфиг | app files |

Формат — **JSON Lines**, по одному объекту на строку, append-only. Ротация по 50 MB,
старые — в zip, не удалять без спроса.

## Конфигурация

Один JSON, читается при старте движка и при «Apply» из dev-консоли. Приоритет:

```
defaults < config.json < dev-консоль (в памяти) < CLI/adb override (для тестов)
```

Пример:

```json
{
  "backend": "auto",
  "threads": 4,
  "threads_io": 2,
  "gpu_layers": 0,
  "ctx": 4096,
  "kv_type": "q8_0",
  "flash_attn": true,
  "mmap": true,
  "prefill_chunk": 256,
  "expert_cache_mb": 1024,
  "expert_cache_per_layer": true,
  "expert_profile": "models/clover/expert-profile.bin",
  "spec": "ngram",
  "spec_max": 4,
  "cache_bias": 0.0,
  "reserve_mb": 512,
  "log_level": "info",
  "log_prompt": false
}
```

Где лежит на устройстве:

```
/sdcard/Android/data/<package>/files/config.json
/sdcard/Android/data/<package>/files/logs/
```

`adb push config.json /sdcard/Android/data/<package>/files/` работает и на Android 11+;
в приложении — редактор и импорт через SAF. После «Apply» движок перезапускает только
нужную часть (кэш/backend), чат сохраняется.

## Схема записи запроса

```json
{
  "type": "request",
  "ts": "2026-11-07T12:34:56.789Z",
  "run_id": "uuid",
  "app": "0.1.0",
  "git": "abc1234",
  "device": {
    "model": "OnePlus 13",
    "soc": "SM8750",
    "android": "15",
    "ram_class_mb": 24576,
    "thermal": "none"
  },
  "model": {
    "name": "Clover-35B-A3B-Preview",
    "quant": "APEX-I-NanoPlus-V2.1",
    "file_sha256": "30E7EE22...",
    "bytes": 12557370304,
    "n_layer": 40,
    "n_expert": 256,
    "n_expert_used": 8
  },
  "config": { "backend": "cpu", "threads": 4, "ctx": 4096, "kv_type": "q8_0", "expert_cache_mb": 1024 },
  "request": { "prompt_tokens": 1904, "max_tokens": 256, "temperature": 0.7, "seed": 42 },
  "metrics": {
    "ttft_ms": 2100,
    "prefill_tok_s": 120.5,
    "decode_tok_s": 8.3,
    "tokens_out": 256,
    "total_ms": 32900,
    "spec_proposed": 40,
    "spec_accepted": 12,
    "expert_hits": 8123,
    "expert_misses": 2901,
    "expert_hit_rate": 0.737,
    "ssd_read_mb": 4180,
    "page_faults": 10234,
    "peak_rss_mb": 3020,
    "peak_pss_mb": 2800,
    "gpu_alloc_mb": 0,
    "free_ram_mb": 6200,
    "battery_pct": 74,
    "battery_temp_c": 38.1
  },
  "errors": []
}
```

Обязательные поля: `ts`, `run_id`, `device`, `model.file_sha256`, `config`, `metrics`.
Без них запись не считается доказательством.

### Счётчики по слоям (debug)

При `log_level=debug` — массив по слоям:

```json
"layers": [
  { "i": 0, "hits": 7, "misses": 1, "read_mb": 2.1, "ms": 1.8 },
  ...
]
```

Это позволяет видеть, где именно промахи: в начале, в конце, на конкретных экспертах.

## Что считаем

| Метрика | Определение |
| --- | --- |
| **TTFT** | от отправки запроса до первого токена |
| **prefill tok/s** | prompt_tokens / время prefill |
| **decode tok/s** | (tokens_out - 1) / время после первого токена |
| **expert hit rate** | hits / (hits + misses) по всем слоям |
| **ssd_read_mb** | реально прочитано с SSD (не page cache) |
| **peak_pss** | `Debug.getPss()` / `dumpsys meminfo`, пик |
| **gpu_alloc** | сумма наших OpenCL/Vulkan-буферов |
| **thermal** | `PowerManager.getCurrentThermalStatus()` + температура батареи |

Важно: **не смешивать chat и raw**, **не смешивать prefill и decode**, **не сравнивать
разные промпты**. Это разные числа.

## Dev-консоль в приложении

Экран «Developer» (скрытый, открывается 7 тапами по версии):

- backend: `auto / cpu / vulkan / opencl`;
- threads: 1–8 (big/little отдельно);
- gpu_layers: 0…N;
- ctx: 2K/4K/8K/16K/32K;
- kv: `f16 / q8_0 / q4_0`;
- expert cache: MB и «per-layer» вкл/выкл;
- prefill chunk: 64…512;
- spec: off / ngram / mtp;
- cache bias: 0…1;
- reserve: 256…2048 MB;
- кнопки: **Apply**, **Restart engine**, **Run benchmark**, **Export logs**;
- живой график: tok/s, RAM, thermal, hit rate.

«Apply» меняет конфиг и перезапускает движок; чат и модель остаются. «Export logs» —
zip с `config.json`, `session.jsonl`, `runs.jsonl`, `crash.log`, `device.txt`.

## Benchmark из приложения и из adb

- В приложении: выбрать промпт (встроенный `bench/prompt.txt`), число прогонов, задачу
  (`chat`/`raw`), нажать Run. Между прогонами — пауза и проверка thermal.
- Из adb (для автоматизации):

```bash
adb shell am start -n <pkg>/.BenchActivity \
  --es task chat --ei runs 8 --es prompt /sdcard/Android/data/<pkg>/files/prompt.txt
adb pull /sdcard/Android/data/<pkg>/files/logs/runs.jsonl
```

## Правила доказательств

1. Лог **append-only**; не редактируем старые строки.
2. Число без `run_id`, модели, конфига, числа прогонов и задачи — не число.
3. Если результат не воспроизвёлся — пишем «not reproduced» рядом со старым, не затираем.
4. Замеры — на одном бинарнике, в одной сессии, с чередованием A/B.
5. Не мерить во время установки, скачивания модели, зарядки или другого тяжёлого job.
6. Thermal status и температура — обязательны: на телефоне разница 2× из-за нагрева —
   норма.

## Приватность

По умолчанию `log_prompt=false`: тексты промптов и ответов не пишутся. Для отладки можно
включить, но лог становится чувствительным; предупреждаем и не экспортируем автоматически.
