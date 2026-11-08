# Отчёт 06. Полный чек-лист совместимости и 1.3/1.4

Источник: параллельное исследование SnapdragonLLM, 2026-11-08.
Дерево APK: `external/llama-clover-src` (4838ffb). Mainline: `external/llama.cpp` f7591a6.

## TL;DR

- Приложение сейчас — **текстовый autoregressive-чат на CPU**. Vision/audio/video, embedding/rerank, T5/BERT, diffusion/TTS, split-шарды, mmproj, LoRA, GPU/NPU либо не собраны, либо не имеют JNI/UI.
- Главный дефект — **потеря причины ошибки**: `nativeLoad` при сбое возвращает `0`, `Engine.load` кидает `"engine load failed"`, а `nativeLastError` для нулевого handle недоступен (`Engine.kt:110,153-156`). Реальная причина остаётся только в logcat.
- Второй дефект — **слепой автозапуск любого GGUF ≤4 ГБ** (`AppController.kt:110-118`), включая mmproj, embedding, BERT, обрезанный файл.
- Точные несовместимости: `GGML_TYPE_F8_E4M3=43` есть в mainline (`external/llama.cpp/ggml/include/ggml.h`), но отсутствует в vendored `llama-clover-src` (там `GGML_TYPE_COUNT=43`, т.е. id 0..42). В mainline `f7591a6` есть 6 архитектур, которых нет в clover-снапшоте: `xing4_0`, `metis16`, `limite`, `mistral-large4`, `kimi-k31`, `ricursive` (сравнение `src/llama-arch.cpp`).
- mmproj/mtmd не собраны: `LLAMA_BUILD_TOOLS=OFF`, `LLAMA_BUILD_MTMD` не включён (`external/llama-clover-src/CMakeLists.txt:254-269`), хотя `tools/mtmd` в дереве есть.
- Пока не сделан preflight и ModelPack, любая фича выше будет давать «непонятную ошибку» вместо «эта модель не поддерживается».

## 1. Полный список «не работает без доработок»

| ID | Проблема | Причина / где | Минимальная работа | Acceptance | P/Phase |
|---|---|---|---|---|---|
| A1 | GGUF с FP8 (`f8_e4m3`) не грузится | В `llama-clover-src/ggml/include/ggml.h` нет `GGML_TYPE_F8_E4M3`; type 43 неизвестен; `GGML_FTYPE_MOSTLY_F8_E4M3` тоже нет | Либо backport типа/ядер, либо preflight-запрет `E_QUANT_UNSUPPORTED` | Файл отклоняется до `llama_model_load_from_file`, в UI код и тип | P0 / 0-1 |
| A2 | Новые архитектуры | В vendored `llama-arch.cpp` нет 6 имён из mainline (см. TL;DR); `LLM_ARCH_UNKNOWN` → `llama-model.cpp:396-397` | Обновить снапшот/backport или allowlist + `E_ARCH_UNKNOWN` | `metis16`-GGUF даёт понятный отказ, не падение | P1 / 1 |
| A3 | Старые GGUF с удалёнными типами | `Q4_2/Q4_3`, `Q4_0_4_4/8_8`, `IQ4_NL_4_4/8_8` удалены (`ggml.h` комментарии) | Preflight по tensor type ids | `E_QUANT_UNSUPPORTED` | P2 / 1 |
| M1 | Vision не работает | `mtmd` не линкуется; JNI только текст (`snapcore.cpp:393-403`); нет image API | Включить `LLAMA_BUILD_MTMD=ON`, линковать `mtmd`, JNI `nativeLoadMmproj`/`nativeImage` | Qwen2.5-VL + mmproj описывает картинку | P1 / 3 |
| M2 | Audio/Whisper не работает | `mtmd-audio.cpp` есть, но не собрано; нет записи/подачи PCM | `mtmd_bitmap_init_from_audio`, UI-рекордер, JNI | Whisper/voxtral mmproj даёт транскрипт | P2 / 3 |
| M3 | Video не работает | `MTMD_VIDEO` требует `LLAMA_SUBPROCESS` (`tools/mtmd/CMakeLists.txt:5-9`); ffmpeg на устройстве нет | Оставить выключенным, отдавать `E_VIDEO_UNSUPPORTED` | Видео отклоняется с текстом | P2 / 3 |
| M4 | mmproj показывается как чат-модель | `ModelStore.scan` берёт все `*.gguf` (`ModelStore.kt:37-67`); нет фильтра `clip`/`mmproj` | Скрывать `clip.*`/`mmproj` в список ассетов, не autoload | mmproj не выбирается как модель | P0 / 1 |
| N1 | Embedding/rerank/BERT не работают | Всегда `LLAMA_PROCESS_TYPE_DECODE` (`snapcore.cpp:183,547`); нет `llama_encode`, pooling, `llama_set_embeddings`, `llama_get_embeddings` | Отдельный JNI `nativeEmbed`, pooling из `llama_pooling_type`; иначе `E_NON_CHAT` | `nomic-embed` возвращает вектор, не токены | P1 / 3 |
| N2 | T5/T5encoder не работают | Нужны `llama_model_has_encoder` (`llama.h:661`), `llama_encode`, `llama_model_decoder_start_token` (`llama.h:668`); код идёт в decode | Encoder/decoder path в JNI или явный отказ | FLAN-T5 не «висит», выдаёт `E_ENCODER_DECODER` | P1 / 3 |
| D1 | Diffusion LLM (Dream/LLaDA/RND1) | `llm_arch_is_diffusion` есть (`llama-arch.cpp:1283`), но цикл в приложении — один токен за шаг; нет masked-block denoise | Проверить, покрывает ли форк denoise внутри `llama_process(DECODE)`; иначе отдельный цикл | LLaDA даёт осмысленный блок или `E_DIFFUSION_UNSUPPORTED` | P2 / 3 |
| D2 | TTS (qwen3tts/pockettts) | `tools/tts/tts.cpp:91` использует mtmd, `:117` — TTS-prompt; нет audio sink | Не тащить в v1; отказ `E_TTS_UNSUPPORTED` | TTS-GGUF не грузится как чат | P2 / 3 |
| S1 | Split GGUF нельзя выбрать | Загрузчик требует первый шард (`llama-model-loader.cpp:598-650`); JNI передаёт один путь (`snapcore.cpp:263`); `llama_model_load_from_splits` не используется (`llama.h:538`) | Preflight читает `split.count/no`, группирует; JNI — список путей | Выбор `-00002-of-00002` ведёт к первому шарду | P0 / 1 |
| S2 | Нет ModelPack/группировки | `ModelStore.scan` — плоский список; нет лицензий, LoRA, mmproj, SHA | Ввести `pack.json`, каталог на пак | Один пакет = одна карточка | P1 / 1-2 |
| T1 | Нет chat template → молча ChatML | `GgufMeta` не читает `tokenizer.chat_template` (`GgufMeta.kt:38-40`); `common/chat.cpp:907-912` подставляет ChatML | Читать template, показывать имя, давать override; warning при fallback | Base-модель получает предупреждение, не тихий ChatML | P1 / 1 |
| T2 | BOS не ставится | `common_tokenize(..., false, true)` (`snapcore.cpp:435`); `inputs.add_bos=false` (`:163`), хотя vocab может требовать BOS (`llama.h:1225`) | `add_special = llama_vocab_get_add_bos(vocab)` или учёт template | Llama/BERT-подобные промпты не теряют BOS | P1 / 1 |
| T3 | Нет stop/grammar/tool roles | `GenParams` только sampling (`Engine.kt:17-25`); роли только user/assistant (`AppController.kt:34`) | `stop[]`, grammar, `tool`/`developer`, JSON-схема инструментов | Tool-call кейс и `<|im_end|>` не текут в ответ | P1 / 2-3 |
| P1 | Причина ошибки загрузки теряется | `snapcore.cpp:264-268` возвращает 0; `Engine.kt:110` — `"engine load failed"`; `lastError` при handle=0 = `"engine not loaded"` | Глобальный `g_last_load_error` из `android_log_cb`, `nativeLoadEx` → JSON | Любой сбой даёт `code/message/log_tail` | **P0 / 0** |
| P2 | `nativeBegin` может вернуть битый JSON | `snapcore.cpp:507` вставляет `ex.what()` без escaping | Экранировать или собирать JSON-объект | Ошибка шаблона не превращается в `JSONException` | P0 / 0 |
| P3 | Ошибка посреди генерации не видна | `nativeNext` возвращает `null` и при EOG, и при ошибке (`:526-552`); UI смотрит `lastError` только если текст пуст (`AppController.kt:262-266`) | Возвращать sealed-результат/флаг, опрашивать `lastError` каждый токен | «context full» виден при частичном ответе | P1 / 1 |
| P4 | Импорт оставляет битые файлы | Прямая запись в `models/` (`AppController.kt:361-402`), нет `.part`, `StatFs`, магии, SHA | `.part` → atomic rename; проверка magic/free space; удаление при сбое | Прерванный импорт не появляется в списке | **P0 / 0** |
| P5 | Autoload неподходящей модели | `AppController.kt:110-118,142-145` грузит первый ≤4 ГБ | Preflight + `isChatCapable` + явный tap | mmproj/BERT/FP8 не стартуют сами | **P0 / 1** |
| P6 | Квант угадывается по имени | `ModelStore.quantFromName` (`ModelStore.kt:69-82`), `fileType` не используется | Маппинг `general.file_type` + tensor types | APEX/Q4_K_M отображается из метаданных | P1 / 1 |
| P7 | Trim теряет историю без следа | `snapcore.cpp:617-627` чистит messages; UI остаётся | Флаг `history_reset`, пауза генерации, сообщение | После trim чат предупреждает о сбросе | P1 / 1 |
| C1 | ctx 32K на 6 ГБ → OOM | UI даёт до 32768 (`Panels.kt:274-280`); `ctx_wanted` не клампится (`snapcore.cpp:282`); нет оценки | `min(UI, ctx_train, memory budget)` + оценка | 32K на 845 блокируется/кладётся | P1 / 1 |
| C2 | KV-тип молча падает в F16 | `kv_type_from_name` знает только q8_0/q4_0/q5_1 (`snapcore.cpp:151-156`) | Валидировать список, отдавать `E_KV_UNSUPPORTED` | Неизвестный kv не меняет память молча | P1 / 1 |
| C3 | `moe_cache_mb` — no-op на CPU | `Panels.kt:314-318`; docs/14 | Скрыть/пометить «только GPU» | UI не обещает ускорение | P2 / 4 |
| C4 | LoRA не применяется | `llama_adapter_lora_init` (`llama.h:691`) не вызывается | JNI `nativeApplyLora(path, scale)` + ModelPack | LoRA меняет ответ, scale виден в логе | P2 / 2 |
| C5 | GPU/NPU не работают | CMake не включает `GGML_VULKAN/OPENCL`; `GGML_BACKEND_DL=OFF`; `gpu_layers` — no-op; Backend в UI жёстко CPU (`Panels.kt:253-259`); QNN отсутствует | Сборка backend, runtime enumeration, fallback, device gate | `gpu_layers>0` реально грузит слои или отключён | P2 / 4 |
| L1 | Логи не помогают диагностике | Нет `error_code`, `log_tail`, `arch/file_type`, `mmproj`, `split`, `ram_est`; `android_log_cb` не сохраняет последние строки (`snapcore.cpp:69-82`) | Расширить JSONL и native error ring | Один JSONL-файл объясняет сбой | **P0 / 0** |

## 2. Чек-лист 1.4 (UI / JNI / ModelStore / GgufMeta / Controller)

| # | Где | Что сделать | Ошибка / формат | Текст пользователю | Тест |
|---|---|---|---|---|---|
| C1 | `snapcore.cpp`, `EngineNative.kt`, `Engine.kt` | `nativeLoadEx` возвращает JSON `{ok,handle,error:{code,message,detail,log_tail}}`; ring-buffer из `llama_log_set` | `E_LOAD_NATIVE` | «Модель не загрузилась: <detail>. Код: E_LOAD_NATIVE» | Подсунуть битый GGUF → код и хвост лога в UI |
| C2 | `GgufMeta.kt` → `GgufPreflight.kt` | Читать arch, file_type, tensor types, chat_template, tokenizer, split, clip.*, head_count_kv, license | `E_GGUF_*`, `E_ARCH_*`, `E_QUANT_*` | «Формат GGUF не поддерживается: <code>» | Unit-тест на синтетических заголовках |
| C3 | `AppController.selectModel` | Preflight **до** `engine.load`; блокировать unsupported | `E_PREFLIGHT_BLOCKED` | «Эта модель несовместима: <причина>. Загрузить всё равно?» (danger) | FP8/mmproj не доходят до native |
| C4 | `ModelStore.scan` | Группировать шарды, скрывать mmproj, добавлять `fileType`, `capabilities`, `unsupportedReason` | — | Бейджи `vision`, `embedding`, `unsupported` | Список из 3 шардов = 1 карточка |
| C5 | `AppController.refreshModels` | Autoload только `isChatCapable && !isMoeTooBig && preflight.ok` | `E_AUTOLOAD_SKIPPED` | «Модель не загружена автоматически: нажмите Load» | mmproj/BERT не стартуют |
| C6 | `AppController.importModel` | `*.part`, `StatFs`, magic, отмена, atomic rename, SHA-256, запрет перезаписи без подтверждения | `E_IMPORT_*` | «Недостаточно места / файл повреждён / импорт отменён» | Оборвать копирование → файла нет |
| C7 | `ChatScreen.ErrorBanner` | Показывать `code`, короткий текст, раскрываемый `detail/log_tail`, кнопку Copy | — | «Не хватает памяти под KV. Уменьшите ctx до 2048» | Скриншот + copy |
| C8 | `DeviceInfo.kt` | `estimateMb = weights + kv + scratch + 400`; сравнение с `availMem`/`memoryClass` | `E_RAM_LOW` warning | «Нужно ~X МБ, доступно ~Y МБ. Уменьшите ctx/KV» | 35B/32K → предупреждение до load |
| C9 | `Panels.kt` Tuning | `ctx` максимум = `min(ctx_train, budget)`; показывать effective ctx | `E_CTX_CLAMPED` | «Контекст ограничен 4096: модель обучена на 4096» | 32K на Qwen 4K → 4K |
| C10 | `snapcore.cpp` | `add_special` из vocab; template override из pack/UI; `no_jinja` fallback | `E_TEMPLATE_FALLBACK` warning | «В модели нет chat template, используется ChatML» | Base-модель показывает warning |
| C11 | `GenParams`/JNI | Добавить `stop[]`, `grammar`, роли `tool/developer`, `seed=random` | `E_UNSUPPORTED_ROLE` | «Tool-calling не поддержан этой моделью» | `<|im_end|>` не попадает в текст |
| C12 | `snapcore.cpp` trim | Не чистить `messages` без флага; останавливать генерацию; UI-баннер | `E_HISTORY_RESET` | «История сброшена из-за нехватки памяти» | onTrimMemory(80) → баннер |
| C13 | `AppController.logRequest` | Добавить `error_code`, `log_tail`, `arch`, `file_type`, `backend`, `mmproj`, `split`, `ram_est_mb`, `seed`, `stop_reason` | — | — | JSONL-схема в docs |
| C14 | `Prefs.kt` | Убрать рассинхрон `batch` (export 256 / getter 128); читать `min_p`, `repeat`, `stop`, `template` из `config.json` | `E_CONFIG_INVALID` | «Некорректный config.json: <поле>» | Push config → значения применились |
| C15 | `Panels.ModelCardItem` | Показывать `arch`, `fileType`, `ctx_train`, `vision/embedding`, причину unsupported | — | «Не поддерживается: FP8 (type 43)» | Карточка не врёт про CPU/GPU |
| C16 | `Panels` Backend | Не показывать нерабочие Vulkan/OpenCL как выбор; писать «не собрано» | — | «Vulkan/OpenCL отсутствуют в этой сборке» | Клик не меняет backend |

## 3. GGUF preflight и ModelPack

**Preflight** — отдельный Kotlin-объект, читает только заголовок/KV/tensor-info, без данных:

```kotlin
data class PreflightReport(
  val ok: Boolean, val code: String?, val message: String?, val detail: String?,
  val warnings: List<String>, val arch: String?, val fileType: Int?, val quant: String?,
  val nLayer: Int?, val nCtxTrain: Int?, val nEmbd: Int?, val nHeadKv: Int?,
  val splitCount: Int?, val splitNo: Int?, val shards: List<File>,
  val mmproj: Boolean, val caps: Set<String>, // vision,audio,video,embedding,rerank,diffusion,tts
  val chatTemplate: String?, val license: String?, val estimateMb: Int
)
```

Проверки: magic `GGUF`; version 2–3; tensor count/offsets не выходят за файл; `general.architecture`; `general.file_type`; все tensor types ∈ allowlist (сейчас 0..42; отдельно `E_F8_E4M3` для 43); `split.count/no` и наличие соседей; `tokenizer.chat_template`/`tokenizer.ggml.add_bos_token`; `clip.projector_type`/`clip.vision.projector_type`/`clip.audio.projector_type` (`clip-impl.h:35,53,87`); `*.head_count_kv`, `*.context_length`; лицензия. `split.*` в GGUF: `split.no=0`, `split.count=N` (`llama-arch.cpp:299-302`). «Проверить», какие KV продублированы во 2..N шардах, и не полагаться на это.

Формат ошибки:

```json
{"ok":false,"code":"E_QUANT_UNSUPPORTED","message":"FP8 E4M3 не поддержан",
 "detail":"tensor blk.0.attn_q.weight type=43",
 "hint":"Сконвертируйте в Q4_K_M",
 "log_tail":"..."}
```

Оценка памяти: `weights ≈ file bytes` (mmap — file-backed), `KV = n_layer * n_ctx * n_head_kv * head_dim * 2 * bpe`, где `bpe` ≈ 2.0 (f16), 1.0625 (q8_0), 0.5625 (q4_0); `scratch ≈ n_batch * n_embd * 8 * 4`; `+400 МБ` оболочки. Для MoE отдельно помечать «эксперты читаются с UFS», не считать весь файл резидентным.

**ModelPack**:

```json
{
  "schema": 1, "id": "qwen2.5-vl-3b-q4",
  "base": {"file":"base-00001-of-00002.gguf","shards":2,"sha256":"...","bytes":0},
  "mmproj": {"file":"mmproj.gguf","sha256":"...","required":true},
  "lora": [{"file":"adapter.gguf","scale":0.8,"sha256":"..."}],
  "chat_template": "optional override",
  "license": {"spdx":"Apache-2.0","name":"...","url":"..."},
  "capabilities":["vision","text"], "min_ram_mb": 4096,
  "recommended":{"ctx":4096,"batch":128,"kv":"q8_0"}
}
```

Каталог: `models/<id>/pack.json` + base-шарды + mmproj + LoRA. Импорт `.modelpack` (zip) или папки; flat GGUF мигрируется в pack с warning. Валидация: SHA, размеры, split-цепочка, mmproj↔base, лицензия (warning, не блок).

## 4. Тестовая матрица

| # | Модель | Ожидание | Ловим ошибку | Логи |
|---|---|---|---|---|
| 1 | SmolLM2-135M / Qwen2.5-0.5B Q4_K_M | Load, chat, template, BOS | `E_TEMPLATE_MISSING` warning | `arch`, `file_type`, `ctx`, TTFT |
| 2 | Qwen3-4B / Llama-3.2-3B | Load 4K, trim, stop | BOS/stop leak | `n_ctx`, `dropped`, `stop_reason` |
| 3 | Qwen1.5-MoE-A2.7B / Clover 35B | mmap, expert streaming, no OOM | `E_RAM_LOW`, LMK | `moe`, `rss`, `peak_rss` |
| 4 | BitNet b1.58 2B4T TQ1_0/TQ2_0 | Load и ответ | `E_QUANT_UNSUPPORTED` если тип не в сборке | `quant=TQ1_0`, quality gate |
| 5 | Qwen2.5-VL + mmproj | Текст без mmproj = warning; с mmproj = image | `E_MMPROJ_REQUIRED` | `mmproj`, `projector_type` |
| 6 | Whisper/voxtral + audio mmproj | Транскрипт | `E_AUDIO_UNSUPPORTED` | `caps=audio` |
| 7 | MP4 | Отказ | `E_VIDEO_UNSUPPORTED` | `MTMD_VIDEO=off` |
| 8 | nomic-embed / bge / reranker | Вектор/скор, не чат | `E_NON_CHAT` | `pooling`, `embd_dim` |
| 9 | FLAN-T5 | Encode→decode | `E_ENCODER_DECODER` | `has_encoder` |
| 10 | LLaDA/Dream | Осмысленный блок или отказ | `E_DIFFUSION_UNSUPPORTED` | `is_diffusion` |
| 11 | qwen3tts/pockettts | Отказ в v1 | `E_TTS_UNSUPPORTED` | `caps=tts` |
| 12 | Split 2-part | Выбор любого шарда → первый + все | `E_SPLIT_NOT_FIRST` | `split_no/count`, список |
| 13 | FP8 GGUF | Отказ до native | `E_QUANT_UNSUPPORTED` (type 43) | `tensor_type=43` |
| 14 | Обрезанный файл | Отказ, файл удалён | `E_GGUF_TRUNCATED` | `bytes` vs offset |

Для каждого прогона: `adb logcat -s SLLM:*` + `session.jsonl`; в логе обязательны `run_id`, `preflight`, `error_code`, `log_tail`, `backend`, `model.arch`, `model.file_type`, `ram_est_mb`.

## 5. Поэтапный план

### Phase 0 — диагностика (3–5 дней)
- P0: native error ring + `nativeLoadEx`, проброс `code/message/detail/log_tail` в `Engine`/`AppController`/`ErrorBanner`.
- P0: preflight-скелет (magic/version/tensor type/arch), запрет autoload без preflight, атомарный импорт, фильтр mmproj.
- P0: логи `error_code`, `log_tail`, `arch`, `file_type`, `ram_est_mb`.
- **Выход:** любой из 14 кейсов матрицы даёт понятный код и текст; битые/неподходящие файлы не грузятся; есть JSONL-доказательство.

### Phase 1 — модель-менеджер и совместимость (1–2 недели)
- 1.1 Полный preflight: chat_template, BOS/EOS, split, mmproj, KV-параметры, лицензия, memory estimate.
- 1.2 ModelPack v1 + группировка шардов/mmproj, карточки и фильтры.
- 1.3 Autoload/выбор только chat-capable; ctx clamp; KV allowlist; шаблон и BOS; stop/роли.
- 1.4 Ошибки UI: код, hint, раскрываемый detail, Copy; Models panel тоже показывает ошибки импорта.
- **Выход:** матрица 1–3, 12–14 проходит; ни один неподдерживаемый файл не доходит до native; в логе есть причина.

### Phase 2 — ternary и LoRA (1–2 недели)
- 2.1 Проверить TQ1_0/TQ2_0 на BitNet: загрузка, CPU-ядра, качество; бейдж `TQ`.
- 2.2 LoRA JNI (`llama_adapter_lora_init`, `llama_set_adapter_lora`), scale, метаданные; ModelPack.lora.
- 2.3 Quality gate для TQ/LoRA; отключаемо.
- **Выход:** BitNet отвечает; LoRA меняет ответ и логируется; при отсутствии поддержки — `E_QUANT_UNSUPPORTED`/`E_LORA_UNSUPPORTED`.

### Phase 3 — мультимодальность и non-chat (2–4 недели)
- 3.1 `LLAMA_BUILD_MTMD=ON`, линковка `mtmd`, JNI image/audio, подбор mmproj к base, UI-вложения.
- 3.2 Embedding/rerank: `llama_encode`, pooling, `llama_get_embeddings`, отдельный режим UI.
- 3.3 T5/BERT: encoder/decoder path; иначе явные `E_ENCODER_DECODER`/`E_NON_CHAT`.
- 3.4 Diffusion/TTS: исследовать и пометить unsupported, не тащить в чат.
- **Выход:** vision-кейс 5 и embedding 8 проходят; audio/video/TTS честно отказывают; Clover не регрессирует.

### Phase 4 — backends (2–6 недель)
- 4.1 Сборка Vulkan/OpenCL, runtime enumeration, `gpu_layers` реально работает; `moe_cache` только с GPU.
- 4.2 Device gate: 845 = CPU, 8 Gen 3/Elite — отдельные профили; fallback при сбое init.
- 4.3 NPU/QNN — только spike, без обещаний.
- **Выход:** таблица prefill/decode CPU vs GPU; backend виден в логе; на 845 поведение не хуже текущего.

**Правило приёмки:** каждый пункт закрывается только при наличии (а) кода, (б) теста из матрицы, (в) строки в `session.jsonl` с `error_code`/метриками. Иначе это не «сделано», а «кажется, работает».
