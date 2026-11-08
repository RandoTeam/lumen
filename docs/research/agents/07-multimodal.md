# Отчёт 07. Мультимодальная архитектура (полная версия)

Источник: параллельное исследование SnapdragonLLM, 2026-11-08.
Устройство: MI 8, SD845, 6 GB, Android 15.

## TL;DR

Строим не «комбайн из десяти движков», а **одно приложение с одним ядром и сменными пакетами способностей**. Ядро — `libsnapcore.so` (llama.cpp + `libmtmd`), поверх него Kotlin-слой `EngineGate → ModelRuntime → JobQueue → MemoryGovernor`. Тяжёлая модель в процессе **всегда одна**; TTS и MediaCodec — «лёгкие» сервисы без моделей.

Ключевой факт: в пине `4838ffb` уже лежит `tools/mtmd` с поддержкой image/audio/video, SmolVLM, Qwen2.5-VL, Gemma3, Moondream2 и даже Qwen3-ASR. **Переходить на mainline ради Phase B не нужно** — достаточно включить `LLAMA_BUILD_MTMD` и слинковать `mtmd`. Mainline понадобится только если mtmd из ветки не пройдёт device-тесты или потребуется модель/фикс, которых в ней нет.

Рекомендуемый набор под 6 GB: текст Qwen2.5-0.5B (25 tok/s, факт), vision — **SmolVLM-500M/256M** (0.28–0.55 GB), видео — **SmolVLM2-500M-Video** (0.55 GB), ASR — **Qwen3-ASR-0.6B** (1.02 GB, Apache-2.0) или Whisper tiny (75 MB) как lite-пакет, image-gen — **SD1.5 Q4 / SD-Turbo** только как 845-demo. Qwen2.5-VL-3B (2.77 GB) и Gemma3-4B (3.34 GB) на 6 GB **не рекомендую**: пик 3.5–5 GB, LMK рядом.

## 1. Архитектура «ядро + пакеты способностей»

```
┌──────────────────────── Compose UI ────────────────────────┐
│ Chat (+ photo/video/audio/file) · Studio · Voice · Models  │
│ Tuning · Telemetry · Logs                                  │
└──────────────────────────┬─────────────────────────────────┘
                           │ AppController / StateFlow
┌──────────────────────────▼─────────────────────────────────┐
│ EngineGate · JobQueue · MemoryGovernor · CapabilityRegistry│
│ ModelPackStore · Telemetry/JSONL                           │
└──────────────────────────┬─────────────────────────────────┘
        ┌──────────────────┼──────────────────┐
        ▼                  ▼                  ▼
 libsnapcore.so      libsnapdiffusion.so   Android APIs
 llama+ggml+mtmd     SD.cpp (lazy dlopen)   TTS, MediaCodec,
 text/vision/audio   Studio only            MediaMetadataRetriever
```

**Пакеты, а не отдельные приложения.** «Пакет способности» — это модель + companion-файлы + манифест. Он не является Android dynamic feature: APK остаётся один, нативные модули — обычные `.so`, модели — скачиваемые каталоги.

| Компонент | Что внутри | Загрузка |
|---|---|---|
| `core` | `libsnapcore.so`: llama, ggml CPU, common, **mtmd**, stb_image, miniaudio | всегда |
| `text` | GGUF Q4/Q5/Q8, chat template, KV | по выбору модели |
| `vision-in` | тот же `libsnapcore` + mmproj GGUF | mmproj грузится/выгружается отдельно от текстовой модели |
| `audio-in` | Qwen3-ASR-0.6B + audio-mmproj через mtmd; опционально whisper.cpp | по кнопке «микрофон/файл» |
| `audio-out` | `android.speech.tts.TextToSpeech` | без модели, системный движок |
| `image-gen` | `libsnapdiffusion.so` (stable-diffusion.cpp + свой ggml) | lazy, только Studio |
| `video-in` | MediaCodec/MediaMetadataRetriever → кадры → mtmd | без отдельного движка |
| `video-gen` | research, не в 6 GB | позже |

**Почему отдельные `.so` для ASR/SD.** У whisper.cpp и stable-diffusion.cpp свои копии ggml. Если всё статически слить в `libsnapcore.so`, APK и кодовая PSS растут, а конфликты символов/регистраций бэкендов становятся трудноуловимыми. Решение: `libsnapcore.so` — всегда; `libsnapdiffusion.so` — только во «full»-сборке, грузится через `dlopen`/`dlclose`; whisper — только если откажемся от Qwen3-ASR. Скрыть символы (`-fvisibility=hidden`), не линковать две версии llama в один процесс.

**`EngineGate`.** Состояния `IDLE → RESERVED → LOADING → READY → RUNNING → COOLING`. `acquire(runtimeSpec, priority)` возвращает lease или ставит в очередь. Тяжёлый lease ровно один. `release` освобождает mmap/ctx/threadpool. `preempt` выгружает **простаивающие** лёгкие/средние runtime, но никогда не убивает активную генерацию.

**`JobQueue`.** Один worker для тяжёлых задач, отдельный — для лёгких. Приоритеты: P0 «отправить сообщение», P1 «кодировать вложение», P2 TTS, P3 Studio, P4 фоновая докачка. Прогресс по фазам: download → load → encode → prefill → decode. Кооперативная отмена, таймауты (load 120 с, encode 120 с, генерация 600 с), пауза при thermal throttle.

**`MemoryGovernor`.** Оценка пика:

```
peak ≈ model_mb + mmproj_mb + KV(ctx) + compute(batch) + encode_scratch(px) + 250 MB
```

`KV` считается по формуле `2 × n_layer × n_kv_heads × head_dim × bytes × ctx`; `compute` — измеренные 127.5 MiB при batch 128 (факт). `encode_scratch` для vision — 200–400 MB на 512 px, 600 MB–1 GB на 768 px (гипотеза, замерить).

Бюджеты (факт: доступно 3.58 GiB в покое; реалистичный бюджет приложения 2.5–3.2 GiB):

| Устройство | soft / hard бюджет приложения | Правило |
|---|---:|---|
| 6 GB (MI 8) | **2.4 / 2.8 GB** | Qwen2.5-VL-3B и Gemma3-4B не грузить |
| 8 GB | 4.2 / 4.8 GB | 3B VLM возможен, но с оговорками |
| 12 GB | 6.5 / 7.5 GB | 3B VLM + ASR |
| 16 GB | 9 / 10.5 GB | Gemma3-4B, SD1.5 |
| 24 GB | 14 / 16 GB | SDXL/видео-gen research |

`onTrimMemory`:
- `RUNNING_MODERATE` — выгрузить idle diffusion/ASR, очистить encode-scratch;
- `RUNNING_LOW` — выгрузить mmproj и ASR, оставить текст и KV;
- `RUNNING_CRITICAL` — очистить KV, отменить некритичные job’ы, оставить только веса текста;
- `COMPLETE` — сохранить чат, освободить всё.

Сейчас код чистит KV на MODERATE и выше — это слишком агрессивно для vision; пороги нужно разнести (факт по `snapcore.cpp`, гипотеза по оптимальной политике).

**Одновременность на 6 GB:**

| Комбинация | Пик | Вердикт |
|---|---:|---|
| Текст 0.5B + TTS | ~0.6 GB | да |
| Текст 0.5B + ASR Qwen3-0.6B | ~1.7 GB | да |
| Текст 0.5B + SmolVLM-500M | ~1.3–1.8 GB | да |
| Текст 0.5B + SmolVLM2-видео 8 кадров | ~1.3–2.0 GB | да |
| Текст 3B + ASR | ~3.7 GB | нет |
| Текст 3B + vision 3B | — | нет, это одна и та же модель |
| Любая тяжёлая + SD1.5 | >3.5 GB | нет |
| Qwen2.5-VL-3B + 8 кадров | 3.5–5 GB | нет |

**Правило:** «тяжёлая» = пик > 700 MB или > 1B параметров. Максимум одна тяжёлая; средняя ASR/vision-0.5B — одна и только с текстом ≤ 0.5B; лёгкие TTS/MediaCodec — всегда. Никогда не грузить две модели > 1B.

## 2. Совместимость с 4838ffb и зависимости

**mtmd уже есть.** В `external/llama-clover-src` на `4838ffb` присутствуют `tools/mtmd/{mtmd.cpp, mtmd-audio.cpp, mtmd-image.cpp, clip.cpp, mtmd-helper.cpp}`. В `tools/mtmd/CMakeLists.txt` цель `mtmd` линкует `ggml`, `llama`, `vendor::stb`, `vendor::miniaudio`, `vendor::hash`, `vendor::sheredom` — всё in-tree. `vendor/` заполнен, отдельных скачиваний не нужно (факт).

**Как включить.** Сейчас `CMakeLists.txt` собирает `llama` и `llama-common`, а `LLAMA_BUILD_TOOLS=OFF`; mtmd не попадает в сборку. Нужно:

```cmake
set(LLAMA_BUILD_MTMD ON CACHE BOOL "" FORCE)
# ...
target_link_libraries(snapcore PRIVATE llama llama-common mtmd android log)
```

Опция `LLAMA_BUILD_MTMD` описана в Top-level CMake: она добавляет `tools/mtmd` standalone, когда tools выключены (факт). `MTMD_VIDEO` при `LLAMA_SUBPROCESS=OFF` принудительно выключается — это нормально, video-helper нам не нужен.

**API, на который садимся:**
- `mtmd_init_from_file(mmproj, text_model, params)` — mmproj к уже загруженной текстовой модели;
- `mtmd_get_cap_from_file(mmproj)` — capabilities **до** загрузки, для UI;
- `mtmd_helper_get_capabilities(model, mctx)` — vision/audio/video/text_gen/audio_gen после загрузки;
- `mtmd_bitmap_init(nx, ny, rgb)` — кадр; `mtmd_bitmap_init_from_audio(n, pcm_f32)`; `mtmd_bitmap_set_mergeable` — для temporal merge у Qwen-VL;
- `mtmd_tokenize_from_parts` — текст + несколько bitmap’ов;
- `mtmd_helper_eval_chunks` — prefill изображения/аудио;
- `mtmd_context_params.image_max_tokens`, `batch_max_tokens`, `n_threads` — жёсткие лимиты под 6 GB.

**Какие модели поддержаны в этой ветке (факт по `docs/multimodal.md`):** SmolVLM/SmolVLM2, Qwen2/2.5-VL, Gemma3 (1B без vision), Moondream2, InternVL 2.5/3, MiniCPM-V, Pixtral, Llama 4, Gemma 4; audio — Qwen3-ASR, Voxtral, Ultravox, Qwen2.5-Omni, Gemma 4; TTS — Qwen3-TTS, Pocket TTS, Soprano, LFM3-Audio. Для 6 GB практический интерес — только SmolVLM/2, Qwen3-ASR и, экспериментально, Moondream2 в Q4.

**Нужен ли mainline?** Для Phase B — **нет**. Ветка уже содержит mtmd и нужные архитектуры. Mainline нужен, если: (1) mtmd-тесты на устройстве падают, (2) нужна модель/new fix, которых нет в PR #31306, (3) Clover перестаёт быть нужен. Тогда два пути: cherry-pick mtmd-фиксов в ветку либо rebase Clover-патча на свежий mainline. Радикальный fallback — два flavor: `clover` (text-only, pinned) и `stable` (mainline + mtmd). Держать две версии llama в одном процессе нельзя.

**whisper.cpp** — MIT, Android-пример есть, CPU-only, tiny 75 MB / base 142 MB. Но если Qwen3-ASR-0.6B проходит по скорости, отдельный `.so` не нужен: mtmd уже умеет audio input. Whisper оставить как lite-пакет для 2–4 GB устройств или fallback по качеству.

**stable-diffusion.cpp** — MIT, чистый C++/ggml, SD1.5/SDXL/Flux, GGUF-кванты. На Android собирается NDK, CPU. Это отдельный `.so` + свой ggml; грузить только в Studio и только когда больше ничего тяжёлого нет. SD1.5 Q4_0 — 1.57 GB (факт по HF), пик с латентами/VAE — 2.2–2.8 GB.

**Android TTS** — `TextToSpeech`, без модели и без сети при offline-voice. Память — отдельный процесс движка, 30–100 MB. Риск: на LineageOS без GApps может не быть русского голоса; проверять `getVoices()`, `isLanguageAvailable("ru-RU")`, `isNetworkConnectionRequired`. Если голоса нет — честный CTA «установить TTS-движок», а не тишина.

**MediaCodec/MediaMetadataRetriever** — без моделей. 845 декодирует H.264/HEVC аппаратно. Для видео берём 8–12 кадров, не тащим ffmpeg/ExoPlayer/OpenCV.

**Лицензии:**

| Компонент | Лицензия | Действие |
|---|---|---|
| llama.cpp / mtmd | MIT | NOTICE |
| whisper.cpp / Whisper | MIT | NOTICE |
| stable-diffusion.cpp | MIT | NOTICE |
| SmolVLM / SmolVLM2 | Apache-2.0 | можно шиппить как recommended |
| Qwen3-ASR | Apache-2.0 | ок |
| Qwen2.5-VL-3B | **Qwen Research** | не default, показать badge |
| Gemma 3 | Gemma Terms | проверить условия |
| Moondream2 | Apache-2.0 (проверить карточку) | ок |
| SD1.5 | CreativeML OpenRAIL-M | use restrictions |
| SD-Turbo | non-commercial (проверить) | только demo |
| Android TTS | системный API | голоса могут быть proprietary |

## 3. UX/IA

**Навигация.** Пять основных разделов вместо восьми: **Chat · Studio · Voice · Models · Dev**. В `Dev` — Tuning, Telemetry, Logs. Это сохраняет текущую логику, но не превращает шторку в комбайн.

**Chat — хаб.** Слева от поля ввода — «+». Он открывает bottom sheet:
- Фото из галереи;
- Камера (`TakePicture` + FileProvider, без CameraX);
- Файл (image/audio/video);
- Запись аудио.

Каждый пункт имеет состояние: `доступно`, `нужен пакет`, `модель не умеет`, `мало памяти`. Disabled-пункт не молчит, а объясняет и даёт CTA: «Установить vision-пакет (109 MB)», «Переключиться на SmolVLM-500M», «Закрыть Studio и повторить».

Вложение — chip над полем ввода: thumbnail/длительность/размер, крестик, прогресс обработки. В сообщении — превью: image, waveform, первый кадр видео + «8 frames». В ответе — строка `stats`: `encode 4.2 s · prefill 38 tok/s · decode 21 tok/s · 1.4 GB peak`. Кнопка Stop работает на всех фазах, включая encode.

**Состояния, которые обязаны быть в UI:**
- `MODEL_TEXT_ONLY` — «эта модель не видит изображения»;
- `MMPROJ_MISSING` — «нужен companion-файл»;
- `MMPROJ_INCOMPATIBLE` — «mmproj от другой архитектуры»;
- `INSUFFICIENT_MEMORY` — «нужно ~1.7 GB, свободно 1.1 GB»;
- `ENCODING` / `PREFILL` / `GENERATING` / `THERMAL_PAUSE` / `CANCELLED`.

**Models.** Карточка показывает: архитектуру, квант, ctx, размер, лицензию, **badges способностей** (`T`, `V`, `A`, `I`), оценку RAM, «установлено / не хватает companion». Кнопки: Install / Verify SHA / Remove / Set as chat model. Фильтр «для 6 GB». Не показывать десять движков — показывать 3–4 рекомендованных пакета и «все».

**Studio.** Prompt, negative, 512×512, steps 4–20, seed, presets, очередь, галерея, Share/Save. На 6 GB — badge «845 demo», предупреждение о времени и памяти. «Отправить в чат» — результат можно прикрепить и попросить VLM раскритиковать.

**Voice.** Две вкладки: **Listen** (TTS: голос, язык, скорость, «прочитать последний ответ») и **Transcribe** (запись/файл, язык auto/ru/en, transcript preview с редактированием перед отправкой). TTS всегда доступен, даже для текстовой модели. ASR — только если установлен пакет.

**Telemetry/Logs.** Добавить в JSONL: `capability`, `pack_id`, `media{type,count,bytes,encode_ms}`, `phase_timings`, `peak_pss_mb`, `min_avail_mb`, `trim_level`, `thermal_status`. Тексты и изображения не логировать, только хэши/размеры.

## 4. Модели и пакеты

| Пакет | Файлы | Размер | Лицензия | Вердикт для 6 GB |
|---|---|---:|---|---|
| `text-qwen05` | Qwen2.5-0.5B Q4_K_M | ~0.4 GB | Apache-2.0 | база, 25 tok/s (факт) |
| `vision-smol256` | SmolVLM-256M Q8 + mmproj Q8 | 175 + 104 MB | Apache-2.0 | **default vision** |
| `vision-smol500` | SmolVLM-500M Q8 + mmproj Q8 | 437 + 109 MB | Apache-2.0 | **default vision+** |
| `video-smol2-500` | SmolVLM2-500M-Video Q8 + mmproj Q8 | 437 + 109 MB | Apache-2.0 | default video |
| `vision-qwen25vl3` | Qwen2.5-VL-3B Q4 + mmproj Q8 | 1.93 + 0.845 GB | Qwen Research | только 8+ GB / experimental |
| `vision-gemma3-4b` | Gemma3-4B Q4 + mmproj F16 | 2.49 + 0.851 GB | Gemma | нет на 6 GB |
| `vision-moondream2` | Moondream2 f16 + mmproj f16 | 2.84 + 0.91 GB | Apache-2.0 | нет; Q4-кастом возможен |
| `asr-qwen3-06` | Qwen3-ASR-0.6B Q8 + mmproj Q8 | 805 + 214 MB | Apache-2.0 | **default ASR** |
| `asr-whisper-tiny` | ggml-tiny | ~75 MB | MIT | lite/fallback |
| `asr-whisper-base` | ggml-base | ~142 MB | MIT | lite+ |
| `img-sd15-q4` | SD1.5 Q4_0 + VAE + TAESD | ~1.57 GB | OpenRAIL-M | 845 demo / 16 GB |
| `img-sd-turbo-q4` | SD-Turbo Q4 | ~1.6 GB | non-commercial | demo |
| `tts-system` | — | 0 | системная | всегда |

**Ожидаемая скорость на 845 (гипотезы, требуют замера):**

| Сценарий | TTFT / время | Decode |
|---|---|---|
| SmolVLM-256M, 512 px | 2–5 s | 30–40 tok/s |
| SmolVLM-500M, 512 px | 3–8 s | 20–25 tok/s |
| Qwen2.5-VL-3B, 512 px | 30–90 s | 4–6 tok/s |
| Moondream2 Q4, 512 px | 10–30 s | 8–12 tok/s |
| Qwen3-ASR-0.6B, 30 s аудио | 15–45 s | — |
| Whisper tiny, 30 s | 5–15 s | — |
| SD-Turbo, 512 px, 4 steps | 40–90 s | — |
| SD1.5, 512 px, 20 steps | 3–8 min | — |
| Android TTS, первая фраза | 0.2–0.8 s | — |

**`ModelPack` (JSON):**
```json
{
  "id": "vision-smol500",
  "kind": "vision",
  "version": 3,
  "base": "text-smol500",
  "files": [
    {"role":"text","name":"SmolVLM-500M-Q8_0.gguf","size":436806912,"sha256":"..."},
    {"role":"mmproj","name":"mmproj-SmolVLM-500M-Q8_0.gguf","size":108783360,"sha256":"..."}
  ],
  "capabilities": ["vision"],
  "arch": "smolvlm",
  "minRamMb": 1200,
  "recommendedRamMb": 2048,
  "license": "Apache-2.0",
  "companionOf": null
}
```
Установка: `models/packs/<id>/`, `.tmp` → проверка SHA → atomic rename; проверка свободного места `size + 10%`; удаление только файлов пакета; общие base-файлы не трогать. LoRA — отдельная роль `lora`, +50–200 MB, только для текста, не в первой волне.

## 5. Фазовый план

**Phase A — текстовое ядро + надёжность (4–6 недель).** Без мультимодальности.
- A0: baseline 30–60 мин, PSS/thermal/LMK.
- A1: `EngineGate`, `ModelRuntime`, `JobQueue`, `MemoryGovernor`; JNI v2 с отдельными handle.
- A2: `ModelPackStore`, SHA, импорт/удаление, свободное место.
- A3: `CapabilityRegistry` (GGUF + mmproj header), badges, заглушки вложений.
- A4: калибровка бюджета на устройстве; `onTrimMemory` по уровням.
- A5: 60-минутный soak, regression-набор, crash-free.

Выход: текст 0.5B работает, PSS ≤ 0.7 GB, 0 OOM/LMK за 60 мин, загрузка ≤ 3 с, текстовая регрессия 20 промптов без деградации.

**Phase B — image input (3–5 недель).**
- B1: `LLAMA_BUILD_MTMD=ON`, линковка, smoke-test `mtmd` на устройстве.
- B2: пакет mmproj, `mtmd_get_cap_from_file`, проверка совместимости.
- B3: Photo Picker/камера, downscale 512/768, EXIF, RGB.
- B4: `mtmd_tokenize_from_parts` + `mtmd_helper_eval_chunks`; лимит `image_max_tokens`.
- B5: прогресс encode/prefill, отмена, очистка bitmap.
- B6: quality gate на SmolVLM-256M/500M (30 изображений: сцена, текст, график, RU).
- B7: Qwen2.5-VL-3B — только experimental, отдельный порог 8 GB.

Выход: TTFT ≤ 8 с на SmolVLM-500M, пик ≤ 1.8 GB, 100 изображений без падений, текстовая PSS ±5%, VQA-качество не ниже baseline.

**Phase C — audio in + TTS (3–5 недель).**
- C1: Qwen3-ASR-0.6B pack через mtmd; проверка RU/EN.
- C2: `AudioRecord` 16 kHz mono, VAD, max 60 с.
- C3: импорт m4a/aac/opus через `MediaExtractor`+`MediaCodec` → PCM (miniaudio не умеет AAC).
- C4: TTS-обёртка, voice discovery, offline-check, sentence streaming.
- C5: mic/transcript UX, редактирование перед отправкой.
- C6: опционально whisper.cpp tiny как lite-пакет.

Выход: 30 с русской речи ≤ 30 с, WER на 20 клипах, TTS первая фраза ≤ 1 с, память TTS ≤ 100 MB, нет сети.

**Phase D — image generation / Studio (4–8 недель; 16/24 GB основной, 845 demo).**
- D1: `libsnapdiffusion.so`, lazy dlopen, отдельный flavor.
- D2: SD1.5 Q4_0 или SD-Turbo; VAE/TAESD.
- D3: 512×512, steps 4–20, seed, preview.
- D4: очередь, прогресс, отмена, thermal pause.
- D5: галерея, Share/Save, «в чат».
- D6: 845 demo gate: только одна картинка, только без других моделей.

Выход: 512×512 ≤ 90 с (Turbo 4 steps) или ≤ 5 мин (SD1.5 20 steps) — гипотеза; пик ≤ 2.8 GB; без OOM; throttling не более 30%.

**Phase E — video input + video-gen research (4–6 недель).**
- E1: `MediaMetadataRetriever`, 8 кадров, равномерно, без первых/последних 0.5 с.
- E2: `MediaCodec` sequential, 12 кадров, downscale 384/512.
- E3: frame plan, timestamps, JPEG/RGB, recycle.
- E4: SmolVLM2-500M-Video; Qwen — только 8+ GB.
- E5: token cap 1024–1536, авто-снижение до 4 кадров, прогресс.
- E6: video-gen — research на 16/24 GB (Wan/LTX), не продукт.

Выход: 10 с 720p → 8 кадров → ответ ≤ 45 с, пик ≤ 2.0 GB, без ffmpeg/ExoPlayer, видео не копируется в app storage.

## 6. Видео внутри текстового раздела

Сценарий: «+» → Video → picker → метаданные (длительность/разрешение) → если > 30 с или > 1080p, вежливый отказ/обрезка. Дальше:

1. **Frame plan:** 8 кадров по умолчанию, 12 максимум на 6 GB; равномерно + сцены с резкой сменой; пропуск чёрных кадров.
2. **Извлечение:** v0 — `MediaMetadataRetriever.getFrameAtTime(t, OPTION_CLOSEST_SYNC)` на IO-диспетчере, по одному кадру, `Bitmap.recycle()` сразу после конвертации в RGB. v1 — `MediaCodec` последовательно, быстрее и точнее.
3. **Downscale:** 384 px для SmolVLM2, 512 px для Qwen; JPEG 85 или сразу RGB; суммарно < 2 MB.
4. **Промпт:** «Это N кадров одного видео в хронологическом порядке, таймкоды …»; для Qwen — `mtmd_bitmap_set_mergeable(true)`.
5. **Token cap:** `image_max_tokens` + лимит кадров; если преффил слишком дорогой — 4 кадра/384 px и явное «6 GB mode».
6. **Ответ:** с таймкодами; в статистике — `frames`, `encode_ms`, `prefill_tok/s`.

Ограничения: без аудиодорожки, без субтитров, без полного плеера, без видеогенерации. Не тащим ffmpeg (mtmd-video helper всё равно требует ffmpeg/ffprobe — факт), не тащим ExoPlayer/OpenCV/CameraX. Видео читается потоково; в кэш попадают только 8–12 RGB-кадров.

## 7. Риски

| Риск | Вероятность | Мера |
|---|---|---|
| mtmd API ломается / ветка отстаёт | высокая | пин 4838ffb, C-обёртка, device-тесты, план rebase |
| LMK/OOM на 6 GB | высокая | бюджеты, preflight, одна тяжёлая модель, trim-политика |
| Qwen2.5-VL-3B не влезает | высокая | не default; 8+ GB; SmolVLM по умолчанию |
| Маленькие VLM галлюцинируют | средняя | quality gate, дисклеймер, 30 изображений |
| Русский TTS отсутствует на LineageOS | средняя | detect voices, CTA, optional engine |
| SD слишком медленный/греется | высокая | только demo, Turbo 4 steps, thermal pause |
| APK/PSS растёт | средняя | отдельные `.so`, lazy, strip, без ffmpeg/OpenCV/ExoPlayer |
| Дубли ggml/символов | средняя | hidden visibility, один тяжёлый runtime |
| AAC/M4A не читается miniaudio | высокая | MediaCodec → PCM |
| Видео раздувает KV | высокая | 4–8 кадров, token cap, предупреждение |
| Clover не мультимодален | факт | не пытаться; отдельный VLM-пакет |

## Итог

Правильная форма — **текстовый чат-хаб + три пакета способностей** (vision, audio, studio), а не десять движков. На 6 GB честно живут SmolVLM-256M/500M, SmolVLM2-500M-Video, Qwen3-ASR-0.6B и Android TTS; Qwen2.5-VL-3B/Gemma3-4B — для 8+ GB; SD — demo. Главное правило: **одна тяжёлая модель за раз, всё остальное — очередь и явные состояния в UI**.
