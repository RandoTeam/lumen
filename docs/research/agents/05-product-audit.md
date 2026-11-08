# Отчёт 05. Продуктовый и архитектурный аудит (полная версия)

Источник: параллельное исследование проекта SnapdragonLLM, 2026-11-07.

## TL;DR

- **Фундамент хороший:** 60 MB PSS, 25.2 tok/s, persistent threadpool, mmap/lazy, честные JSONL-логи. Это надо защитить регрессионными воротами, а не «улучшать» рефакторингом.
- **Главные пробелы v0.1 — не красота, а надёжность:** нет foreground service, чат не сохраняется, `Stop` может блокировать UI до 26 с, два быстрых тапа по модели запускают два `nativeLoad`, импорт пишет сразу в финальный файл без SHA и проверки места.
- **Мультимодальность делать «пакетами способностей»:** текст — ядро; vision/audio/image-gen — отдельные `.so` + модели, загружаемые лениво и выгружаемые по памяти. На 6 GB одновременно живёт только один тяжёлый движок.
- **Нельзя тащить Room/Hilt/Navigation/Coil «для удобства»** — это +2–4 MB кода, startup и PSS. Для истории хватит JSONL/SQLite без Room.
- **Порядок работ:** надёжность и менеджер моделей → UX чата → vision-in → аудио → генерация изображений только на 16/24 GB.

## Аудит: что хорошо

| Что | Почему не ломать |
|---|---|
| 60 MB PSS оболочки (`docs/11-APP.md`) | Один процесс, 5 зависимостей (`app/build.gradle.kts:99-112`), R8 + shrinkResources, только `arm64-v8a`, `extractNativeLibs=false`. Любая новая библиотека — только с замером PSS. |
| Persistent threadpool (`snapcore.cpp:304-323`) | Даёт 25.2 против 18.2 tok/s. Любой новый контекст обязан получать `llama_attach_threadpool`. |
| mmap + lazy + extra bufts (`snapcore.cpp:252-258`) | 12 GB Clover не копируется в RAM; страницы вытесняемы. Не заменять на `load_mode=NONE`. |
| `batch=128`, `n_ubatch≤256` (`snapcore.cpp:283-285`) | Compute buffer 127 MiB вместо 155. Не поднимать без замера. |
| Логи во external files + JSONL (`data/DeviceInfo.kt:62-84`) | `adb pull` без root-танцев, тексты промптов не пишутся, ротация 8 MB. |
| Tuning + `config.json` | Управление без переустановки — ключевая фича проекта. |
| 4 GB-порог автозагрузки (`AppController.kt:110-118`) | Не мапит 12 GB без явного тапа. Сохранить. |
| `utf8_cache` в native (`snapcore.cpp:562-569`) | В Kotlin не приходят рваные UTF-8 — не переписывать на «строку на токен». |
| `state_bytes` считается один раз (`snapcore.cpp:327`) | Правильно: `llama_state_get_size` нельзя звать на токен. |

## Аудит: что плохо (с путями)

### Надёжность и конкурентность

- `AppController.kt:154-202`: `selectModel` не блокирует повторный вызов в состоянии `LOADING`. Два тапа → два `engine.load` → второй `free()` может освободить ручку первого; утечка 12 GB или use-after-free. Нужен single-flight `Mutex`/актор.
- `snapcore.cpp:572-579`: `nativeCancel` берёт `e->lock`, который держит `nativeNext` (до 1.7 с на Clover) или весь prefill (`snapcore.cpp:485-491`, до 26 с). `stop()` вызывается с main-потока (`ChatScreen.kt:684-686`) → заморозка UI/ANR. Нужен `std::atomic<bool>` и проверка отмены внутри prefill-цикла.
- `snapcore.cpp:611-628`: при `TRIM_MEMORY_MODERATE` (60) вызывается `clear_kv` без остановки генерации. Если триггер придёт в фоне, `n_past=0`, а decode продолжит писать в KV с позиции 0 → порча контекста. Трим должен откладываться до конца генерации.
- `AppController.kt:296-304, 306-329`: `unloadModel` и `updateConfig` вызывают `engine.free()` **на main** — ждут нативный lock и размапливают гигабайты. ANR.
- Нет foreground service (`AndroidManifest.xml` пуст по сервисам): экран погас/приложение в фоне → генерация замирает или процесс убивают. Для 26-секундного prefill это гарантированная потеря.
- `AppController.kt:361-402`: импорт пишет напрямую в финальное имя, без `.part`, без `StatFs`, без SHA, без отмены, без проверки расширения (`MainActivity.kt:38` — `*/*`). Обрыв → `ModelStore.scan` видит обрезанный GGUF как модель.
- `data/GgufMeta.kt:87-93`: `ByteArray(len.toInt())` до 1 GB из недоверенного файла — OOM при скане.
- `AppController.kt:343-350`: `trimNow` доступен из Tuning/Telemetry во время генерации — ручной вызов ломает KV.
- `data/Prefs.kt:12-38`: `config.json` перечитывается при каждом старте и перезаписывает SharedPreferences. Tuning «применился», но после перезапуска старый файл снова победил. Плюс `exportJson` (`Prefs.kt:43`) по умолчанию `batch=256`, а рабочий дефолт — 128.

### UX/архитектура

- `ChatScreen.kt:456-583`: нет copy/regenerate/edit/branch/search/selection; markdown — только ```-блоки; незакрытый блок во время стриминга рендерится как код. Нет `SelectionContainer`.
- Нет истории чатов: `messages` живут в `Application` (`AppController.kt:53`), процесс умер — всё lost. `COMPLETE`-трим чистит native-сообщения (`snapcore.cpp:617-623`), а UI-список остаётся → тихая потеря контекста.
- `ChatScreen.kt:115-120`: вечный цикл `refreshMemory()` каждые 2 с, даже когда чат простаивает; обновляет state, который читает Composer → recomposition каждые 2 с.
- `Panels.kt:499-506`: Telemetry каждую секунду дергает `liveStats()` → JNI `stats_json` под тем же `e->lock`, что и decode. На Clover это регулярные паузы.
- `Panels.kt:686-741`: логи читаются целиком (`shopLogsText`, 8 MB) и рендерятся одним `Text` в `verticalScroll`; `lineSequence().count()` в подзаголовке. Джанк и пик PSS. Share уходит через `EXTRA_TEXT` — риск Binder-лимита.
- `ModelsPanel`: нет удаления, переименования, SHA, свободного места, оценки «влезет ли», отмены импорта. `onResume` (`MainActivity.kt:22-25`) каждый раз пересканирует каталог и парсит GGUF-заголовки.
- `TuningPanel`: ctx до 32768 на 6 GB без оценки KV; threads до 8 при замерах 19.4 против 26.6 tok/s; нет per-model профиля, сброса, предупреждения о reload.
- `Theme.kt:21`: `Faint #586274` на `#07080A` — контраст ниже 4.5:1 для 10 sp; тапабельные зоны 38–42 dp; у Canvas-иконок нет contentDescription; строки захардкожены в Kotlin, локализации нет.
- `CMakeLists.txt`: ядро жёстко прибито к `external/llama-clover-src` (PR #31306, `llama_process`/`LLAMA_PROCESS_TYPE_DECODE`). Это блокирует upstream `mtmd` (vision) и новые модели. Нужно разделение mainline/clover до мультимодальности.

## Архитектура мультимодальности

Принцип: **одна оболочка, много «пакетов способностей»**. Пакет = отдельный `.so` + набор моделей + экран/секция UI; грузится по первому использованию, выгружается по таймеру или `onTrimMemory`.

```
core/        EngineGate (single-flight), ModelRuntime (арбитр памяти), JobQueue
text/        llama.cpp (mainline)          Qwen3-0.6/1.7B, Llama-3.2-1/3B
vision-in/   libmtmd + mmproj              SmolVLM-500M (6 GB), Moondream2, Qwen2.5-VL-3B (16 GB)
image-gen/   stable-diffusion.cpp          SD1.5 Q4 + TAESD (Studio; 845 — demo)
audio-in/    whisper.cpp                   tiny/base q5, 30-секундные чанки
audio-out/   Android TextToSpeech          системный офлайн-голос, 0 MB
video-in/    MediaCodec → 8–16 кадров 384–512 px → VLM
```

- **Бэкенды:** CPU/NEON — дефолт; Vulkan — только 8 Gen 3/Elite после замера; QNN/NPU — позже и не в критическом пути; MediaCodec — видео; системный TTS — озвучка. OpenCL на 845 не возвращать (`docs/14-BACKENDS.md`).
- **Ленивость:** `mmproj` грузится только при вложении картинки; whisper — только на время записи; SD — только в Studio. На 6 GB одновременно один тяжёлый движок: перед SD выгружать KV текстовой модели (mmap оставить), после 60 с простоя — `free`.
- **UI:** не плодить вкладки. Chat получает кнопку «+» (фото, файл, аудио), Studio появляется только при установленном image-gen пакете, Models показывает «Capabilities» с размерами пакетов. Так оболочка остаётся текстовой, а не комбайном.
- **Бюджет:** shell ≤60 MB + до 20 MB на миниатюры; модель считается отдельно: weights (mmap) + KV + compute + mmproj. Арбитр обязан оценивать сумму и резервировать 500–600 MB под ОС.

## Модель-менеджер

- **Импорт:** `OpenMultipleDocuments`, фильтр `.gguf`; копирование в `models/<slug>/file.part` → `fsync` → atomic rename. `DigestInputStream` считает SHA-256 за один проход. `StatFs`: нужно `fileSize × 1.15 + KV + compute`, иначе отказ с числами. Отмена и очистка `.part`.
- **Пакеты:** `ModelPack { kind, files[{role, sha, bytes}], license, sourceUrl }`; VLM = model + mmproj, SD = unet + vae + clip. Текущий плоский `ModelStore.scan` этого не умеет.
- **Реестр:** `models.json` с атомарной записью, кэш метаданных по `path+size+mtime`, чтобы `onResume` не парсил заголовки. Скан — только реконсиляция.
- **Удаление:** с подтверждением, размером и блокировкой активной модели.
- **Профили `model+device`:** ключ `sha256 + SOC_MODEL + RAM class + backend`; хранить `ctx/batch/threads/kv/flash/gpuLayers` + замеренные TTFT/decode/thermal. При первом запуске — 20–30 с калибровки (или «пропустить»), кэш в `profiles.json`. Tuning показывает «профиль этой модели», кнопки Reset/Re-calibrate.
- **Старт:** онбординг спрашивает модель; автозагрузка только ≤2 GB и по явному тумблеру; после краха — safe mode (ctx 2048, batch 64).

## Надёжность

- **`GenerationService`** (foreground, `specialUse`): уведомление с моделью, tok/s, progress и Stop; `PARTIAL_WAKE_LOCK` только на генерацию; `startForeground` за 5 с; Activity подписывается на `StateFlow`. Это одновременно решает freeze в фоне и даёт нормальный прогресс.
- **Трим:** `pendingTrim`, применять после генерации; на COMPLETE — sampler+KV, модель оставить mapped; `malloc_trim` для возврата native-страниц.
- **Краши:** писать `request_started` до prefill и `request_end` после; при старте искать «висящий» run_id и предлагать Retry. `Thread.setDefaultUncaughtExceptionHandler` → `crash.jsonl`; нативный SIGSEGV — минимальный async-signal-safe дамп.
- **Чаты:** `chats/<id>.jsonl` + индекс; автосейв после каждого ответа; восстановление, rename/delete/pin, экспорт Markdown/JSON. Для поиска — SQLite FTS5 без Room.
- **Логи:** шаринг файлом через `FileProvider` + `EXTRA_STREAM`, просмотр — постранично, фильтр по `type/run_id`, `pss_mb` и thermal в каждой записи. Приватность: `INTERNET` не добавлять без явного решения; промпты не логировать; чаты — во внутреннем `filesDir`.

## UX-полировка

- **Онбординг:** 3 экрана — «что это/приватность», «выбери модель» (рекомендованные с размером и прогнозом RAM), «откалибровать» (skip).
- **Пустые состояния:** Chat уже хорош (`ChatScreen.kt:316-376`); Models — «импортировать/скачать»; Logs — «пока нет запросов»; Studio — «сгенерируй первое изображение».
- **Ошибки:** маппинг ENOSPC/OOM/LMK/bad GGUF/ctx overflow в человеческий текст + действие («освободить место», «уменьшить ctx», «переимпортировать»), run_id в «details». Сейчас показывается сырой `t.message`.
- **Прогресс:** фазы загрузки (open → map → ctx → warm), байты; prefill «128/512 tok · ETA 12 s»; decode tok/s; импорт % + SHA; генерация изображения step x/y с превью. Для 12 GB это обязательно.
- **Стриминг/отмена:** отдельный буфер стрима, markdown один раз по завершении; Stop отзывается ≤1 токен/чанк; copy, regenerate, edit-and-rerun, branch, share, «поиск по чату» с переходом к сообщению.
- **Дефолты:** threads = число big-ядер (4 на 845, 6 на 8 Gen 3), ctx 4096, batch 128, q8_0, mmap on, mlock off, flash on, max 512; системный промпт на языке пользователя; тёмная тема по умолчанию, но с следованием системе; RU/EN ресурсы; 48 dp, contentDescription, контраст.

## Производительность оболочки

Что незаметно съедает PSS/CPU:
- стриминг перерисовывает весь растущий `Text` каждые 140 мс (`AppController.kt:246-256`) → O(n²) измерений текста; на 845 это отбирает ядро у сэмплера;
- `refreshMemory` каждые 2 с и `liveStats` каждую 1 с → recomposition и contention на native lock;
- бесконечные анимации (`Components.kt:268-281`, `ChatScreen.kt:535-545`) крутят Choreographer, пока экран виден;
- логи одним `Text` на 200k символов;
- импорт обновляет `importProgress` на каждый мегабайт;
- `onResume`-скан с парсингом GGUF.

Контроль: `collectAsStateWithLifecycle` + `repeatOnLifecycle`, пауза polling вне экрана; `LazyColumn` с `contentType` и `key`; `@Immutable` модели; `derivedStateOf`; чтение лога с конца файла; baseline profile через Macrobenchmark (cold start, send, scroll), а не дефолтный AGP; R8 keep уже корректный (`app/proguard-rules.pro`), добавить `androidx.profileinstaller`; `dumpsys meminfo` PSS в CI-скрипт; StrictMode/LeakCanary только в debug; Perfetto на prefill/стриминг.

## Roadmap

| Горизонт | Задача | Impact | Effort |
|---|---|---|---|
| Сейчас (1–2 нед.) | Single-flight движка, atomic cancel, отмена внутри prefill, не чистить KV при генерации | P0 | M |
| | `GenerationService` + уведомление + wake lock | P0 | M |
| | Сохранение чатов (JSONL) + автосейв + восстановление | P0 | M |
| | Импорт: `.part`, место, SHA, отмена, multi-select | P0 | M |
| | Логи: постраничный просмотр, share файлом, пауза polling | P1 | S |
| | Copy/regenerate/search, маппинг ошибок, онбординг | P1 | M |
| | Удаление моделей, профиль на модель, запрет автозагрузки >2 GB | P1 | M |
| | Ворота регрессии: PSS/старт/tok/s на устройстве | P0 | S |
| Месяц | Реестр моделей/пакетов, кэш метаданных, калибровка, `model+device` профили | P1 | L |
| | История чатов, FTS-поиск, ветвление, заголовки, markdown/код | P1 | L |
| | Vision-in: mainline `mtmd` + SmolVLM-500M, вложения, ленивый mmproj | P1 | L |
| | Whisper ASR + системный TTS, разрешения и приватность | P2 | M |
| | Baseline profile, Macrobenchmark, бюджет PSS в CI | P1 | M |
| | Разделение mainline/clover сборок | P1 | M |
| После 16/24 GB | SD1.5/SDXL-Turbo, Studio и очередь задач | P2 | L |
| | Qwen2.5-VL-7B, ctx 32K, два движка одновременно | P2 | L |
| | Видео: кадры+VLM; генерация — облако/оффлоад | P2 | L |
| | Vulkan/QNN на 8 Gen 3/Elite, экспертный кэш Clover | P2 | L |

## 10 самых опасных мест

1. **Гонка `selectModel`** (`AppController.kt:154`) — параллельные `nativeLoad`, утечка 12 GB/UAF.
2. **`nativeTrim` при 60 во время генерации** (`snapcore.cpp:624-627`) — сброс KV без остановки decode.
3. **`Stop` под мьютексом** (`snapcore.cpp:572`, `485-491`) — UI висит до 26 с на Clover; prefill не отменяем.
4. **Нет foreground service** — фон/экран off убивает генерацию, чат теряется.
5. **`engine.free()` на main** (`AppController.kt:296-329`) — ANR на размапливании гигабайтов.
6. **Импорт в финальный файл без места/SHA** (`AppController.kt:374-388`) — обрезанная «модель» и ENOSPC.
7. **`GgufMeta.readString` без лимита** (`GgufMeta.kt:87-93`) — OOM на крафченом GGUF.
8. **COMPLETE-трим чистит native-историю, UI — нет** — тихая потеря контекста.
9. **Логи одним `Text` + `EXTRA_TEXT`** (`Panels.kt:686-741`) — джанк, пик PSS, Binder-лимит.
10. **`config.json` перекрывает Tuning при каждом старте** (`Prefs.kt:12-38`) — «настройки не сохраняются».

Плюс два «почти в топ-10»: ctx 32768/threads 8 без бюджета на 6 GB (LMK/троттлинг) и жёсткая привязка к Clover-форку, блокирующая `mtmd`.

## Метрики успеха

| Метрика | Цель | Как мерить |
|---|---|---|
| PSS оболочки | ≤60 MB idle, ≤75 MB после 20 сообщений, +5 MB — стоп-кран | `dumpsys meminfo` 10 мин idle |
| Cold start | ≤1.5 с до интерактивного экрана | Macrobenchmark |
| Qwen 0.5B | ≥25 tok/s decode, ≥45 prefill — без регрессии >5% | тот же протокол `07-BENCHMARK.md` |
| Clover | ≥0.6 tok/s, TTFT не хуже 26 с | 3 прогона, медиана |
| Sustained 30 мин | без LMK, thermal ≤2, деградация ≤20% | Telemetry + `thermalservice` |
| Stop latency | ≤1 токен (≤200 мс на prefill-чанк) | instrumented test |
| Потеря чата | 0 сообщений при `kill -9` во время генерации | monkey/ADB test |
| Импорт 12 GB | прогресс ±1%, отмена, SHA, нет `.part` в списке | ручной сценарий |
| Логи | открытие ≤150 мс, ни одного кадра >50 мс | Perfetto |
| Idle CPU | ~0%, ≤1 пробуждение/мин | `dumpsys batterystats` |
