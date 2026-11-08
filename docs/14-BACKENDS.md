# 14 — Бэкенды и потоки на Snapdragon 845

Все числа сняты на MI 8, Qwen2.5-0.5B-Instruct Q4_K_M, 2026-11-07.
Один бинарник на серию, `llama-bench`, 2–3 повтора, медиана/среднее.
Это не сравнение с 8 Gen 3: другой GPU, другая память, другой драйвер.

## Итог

| Бэкенд | prefill pp128, tok/s | decode tg32, tok/s | Вердикт |
| --- | ---: | ---: | --- |
| **CPU, 4 потока (по умолчанию)** | **41.8** | **26.6** | лучший на 845 |
| CPU, 3 потока | — | 19.3 | мало |
| CPU, 2 потока | — | 13.5 | мало |
| CPU, 6 потоков | — | 19.4 | big-ядра заняты, A55 мешают |
| CPU, 8 потоков | — | 19.7 | то же |
| CPU + `armv8.2-a+fp16` | 42.4 | 25.8 | в пределах шума, не берём |
| OpenCL (Adreno 630) | 3.05 | 2.23 | в 12–14 раз медленнее |

## CPU

- 4×A75 @ 2.8 GHz — оптимум. 4 потока ровно по числу big-ядер.
- Больше 4 — планировщик добавляет A55 и/или мешает big-ядрам; скорость падает.
- Dotprod нет, поэтому квантованные ядра идут по обычному NEON-пути.
- `GGML_CPU_REPACK=ON` уже включён (runtime repack Q4_0 → Q4_X_X).
- FP16-векторная арифметика (`HAVE_FP16_VECTOR_ARITHMETIC`) собирается,
  но на Q4_K выигрыша нет: +1% prefill, −3% decode. Не включаем в релиз.

## Критичная находка: persistent threadpool

Без явного `llama_attach_threadpool` приложение выдавало **18.2 tok/s**,
а тот же бинарник в CLI — **26.6 tok/s**. `top -H` показывал один поток на 92% CPU:
ggml создавал и уничтожал worker-пул на каждый граф.

После `ggml_threadpool_new` + `llama_attach_threadpool`:

- приложение: **25.2 tok/s**, prefill 48 tok/s, TTFT 1.3 s;
- это практически паритет с CLI.

Код в `app/src/main/cpp/snapcore.cpp`, функция загрузки контекста.
Правило: любой новый контекст должен получать постоянный threadpool.

## OpenCL

Собрано из 4838ffb с `GGML_OPENCL=ON`, NDK glslc, заголовками Khronos
OpenCL-Headers и `libOpenCL.so`, вытащенным с устройства.

Что видит:

```
platform: 'QUALCOMM Snapdragon(TM)'
device:   'QUALCOMM Adreno(TM) (OpenCL 2.0 Adreno(TM) 630)'
```

Adreno-специфичные ядра **не компилируются** драйвером:

```
error: implicit declaration of function 'dot_acc_sat_4x8packed_ss_int'
warning: unknown attribute 'qcom_wave_pair_mode' ignored
```

С `GGML_OPENCL_USE_ADRENO_KERNELS=OFF` собираются generic-ядра, но:

- pp 3.05 tok/s (в 14 раз медленнее CPU);
- tg 2.23 tok/s (в 12 раз медленнее CPU).

Вывод: на Adreno 630 OpenCL-путь для LLM бессмысленен. Оставляем CPU.
Это не приговор для 8 Gen 3 / 8 Elite: там другой драйвер и другая память,
но и там нужен собственный замер.

## Vulkan

Не собран: нет Vulkan SDK / SPIRV-Headers на хосте. NDK glslc есть, но
`find_package(Vulkan COMPONENTS glslc)` требует SDK. На Adreno 630 после
результата OpenCL ожидание низкое; на 8 Gen 3/Elite — отдельная задача.

## Что это значит для конфига по умолчанию

```json
{
  "threads": 4,
  "batch": 128,
  "kv": "q8_0",
  "flash_attn": true,
  "mmap": true,
  "gpu_layers": 0,
  "moe_cache_mb": 0
}
```

- `batch 128` — компромисс: compute buffer 127 MiB вместо 155 MiB при 256;
  prefill 48 tok/s достаточен для чата.
- `gpu_layers 0` на 845; на 8 Gen 3/Elite пересчитать.
- `moe_cache_mb` на CPU — **не no-op, а ошибка**: `llama_moe_cache` требует GPU/IGPU
  и бросает `MoE cache requires a GPU backend`. Держать 0; включать только с
  рабочим GPU-бэкендом.

## Повторить

```powershell
# CPU
.\scripts\bench-cli.ps1 -Threads 4

# OpenCL (если собран)
adb shell "cd /data/local/tmp/sllm-opencl && LD_LIBRARY_PATH=. ./llama-bench -m qwen.gguf -t 4 -p 128 -n 32 -ngl 99"
```
