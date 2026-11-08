# Отчёт 01. Совместимость GGUF (полная версия)

Источник: параллельное исследование проекта SnapdragonLLM, 2026-11-07.
Дерево APK: `external/llama-clover-src` = llama.cpp PR #31306 @ `4838ffb`.
Mainline: `external/llama.cpp` f7591a6. Устройство: MI 8, SD845, 6 GB.

## TL;DR

1. **APK собран ровно из `external/llama-clover-src` = 4838ffb.** В этом дереве 167 архитектур `LLM_ARCH_*`, 165 из них имеют класс и mapping в `llama_model_mapping()`; без реализации остались `GPTJ` и `UNKNOWN`. `clover_moe`支持 нативно, без патчей проекта.
2. **Mainline `f7591a6` новее на 6 архитектур** (`xing4_0`, `metis16`, `mistral-large4`, `kimi-k31`, `limite`, `ricursive`) и на новый tensor type `F8_E4M3` (id 43). GGUF, сделанный новым mainline-квантователем, APK может не открыть.
3. **Кванты поддержаны широко:** Q2_K–Q6_K, Q8_0, IQ1_S/M–IQ4_XS, TQ1_0/TQ2_0, BF16/F16/F32, MXFP4/NVFP4 и новые Q1_0/Q2_0. Q8_K/Q8_1 — промежуточные типы, не для весов.
4. **Ограничивает не только архитектура:** приложение текстовое, mtmd/mmproj не линкуется; `n_gpu_layers=0`; 6 GB RAM; ctx 4096 и max_tokens 512 по умолчанию; при отсутствии chat template молча подставляется ChatML; BOS принудительно выключен.
5. **«Любой .gguf» — нет.** Неподдерживаемая архитектура/тип тензора падает на загрузке, но UI показывает только `engine load failed`, потому что native-ошибка теряется.
6. **Vision/audio/embedding/сырые base-модели** загрузятся в части случаев, но нормального чата не дадут.
7. **По памяти:** 0.1–3 GB — комфортно, 4–5 GB — на грани, 8–12 GB — только mmap-демо с чтением с UFS (Clover 11.69 GB уже так работает: ~0.6 tok/s decode).
8. **Ternary — ниша:** BitNet/TQ-модели надо отдельно проверять по формату (официальный BitNet может быть I2_S, а не TQ1_0/TQ2_0).

## Таблица совместимости

| Семейство | Статус | Причина | Что проверить |
|---|---|---|---|
| Llama 2/3.x, TinyLlama, SmolLM2, Mistral 7B dense, Mixtral | **works** | `LLM_ARCH_LLAMA`; Mixtral — это llama-граф с expert-тензорами, отдельной arch нет | память для 8x7B |
| Qwen 1/2/2.5, Qwen3 dense | **works** | `qwen`, `qwen2`, `qwen3` | шаблон и BOS |
| Qwen2MoE, Qwen3MoE | **works** | `qwen2moe`, `qwen3moe` | 14–30B не влезут в 6 GB без mmap-трэшинга |
| Qwen3-Next, Qwen3.5, Qwen3.5-MoE | **likely** | `qwen3next`, `qwen35`, `qwen35moe`; гибридные/recurrent слои, CPU-ядра есть | реальный прогон, память state |
| Qwen2-VL, Qwen3-VL, Qwen3-VL-MoE | **needs work** | текстовая часть, вероятно, пойдёт; vision требует mmproj/mtmd, которого нет | text-only GGUF без mmproj |
| Qwen4-exp | **needs work** | экспериментальная arch, мало тестов | не обещать пользователю |
| Gemma 1/2/3 | **works** | `gemma`, `gemma2`, `gemma3`; Gemma 3 — только текст | BOS и chat template |
| Gemma 3n | **likely** | `gemma3n`, per-layer embeddings/AltUp; audio-часть не нужна | CPU-операции, память |
| Gemma 4 / 4-assistant | **likely** | arch есть, но модели очень новые и крупные | наличие GGUF и RAM |
| Phi-2/Phi-3/Phi-4 | **works** | `phi2`, `phi3` | — |
| PhiMoE | **works** | `phimoe` | 3.5B+ RAM |
| DeepSeek V2/V3/R1 | **works** | `deepseek2` | огромные MoE, только mmap-демо |
| DeepSeek 3.2/4/4.1 | **likely** | `deepseek32`, `deepseek4`, `deepseek41`; новые MLA/DSV4-пути | CPU-операции, стабильность |
| Command-R | **works** | `command-r` | Command-R+ не для 6 GB |
| Cohere2 / Cohere2-MoE | **works / likely** | `cohere2`, `cohere2moe` | MoE-память |
| OLMoE, OLMo 2 | **works** | `olmoe`, `olmo2`; OLMoE-1B-7B Q4 ≈ 4.2 GB — хороший MoE-тест | реальный RSS |
| DBRX | **works** | `dbrx` | 132B — не для телефона |
| Falcon, Falcon-H1 | **works / likely** | `falcon`; `falcon-h1` гибрид | H1 на CPU |
| Mamba/Mamba2/Jamba, Nemotron-H | **likely** | recurrent/hybrid archs есть, CPU-ядра SSM есть | стабильность и template |
| RWKV6/7 | **works с оговоркой** | arch есть, но обычно нет chat template | качество чата |
| BitNet 1.58 | **likely** | `bitnet`; реально нужны TQ1_0/TQ2_0 | формат конкретного GGUF |
| LLaVA (llama + clip) | **needs work** | отдельной arch нет; это llama + mmproj | текст без картинок возможен, vision — нет |
| Whisper / Qwen-Audio / TTS (qwen3tts, talkie, pockettts) | **unsupported** | Whisper в llama.cpp нет; TTS не чат, нужен audio-пайплайн | не импортировать |
| BERT/эмбеддинги/реранкеры, T5 | **unsupported для чата** | нет causal-LM чата, шаблон подставится ChatML | только embeddings-сценарий |
| Diffusion LLM (dream, llada, llada-moe) | **needs work** | arch есть, но app-цикл — обычный AR decode | отдельный режим семплинга |
| Archs после 4838ffb | **unsupported** | кода нет в APK | пересборка из mainline |
| GGUF с `F8_E4M3` | **unsupported** | тип 43 появился после 4838ffb | пересборка |

## На чём основаны выводы (точные места)

- Список архитектур: `external/llama-clover-src/src/llama-arch.h:14-180`; строковые имена: `src/llama-arch.cpp:9-177`.
- Mapping и ошибки: `src/llama-model.cpp:43-377`, `:375-376` — `unsupported model architecture`; `:394-398` — `unknown model architecture`.
- `clover_moe`: `src/llama-arch.cpp:48`, `src/llama-model.cpp:349-350`, `src/models/models.h:2922-2943`, реализация `src/models/clover-moe.cpp`. В 4838ffb загружаются block-diffusion тензоры (`ssm_bd_*`), которых нет в mainline-версии.
- Кванты: `ggml/include/ggml.h:389-434` (enum, `GGML_TYPE_COUNT=43`); `ggml/src/ggml.c:640-935` (type traits; у Q8_K/Q8_1 нет `to_float`); `ggml/src/ggml-cpu/ggml-cpu.c:215-412` (vec_dot для всех реальных типов).
- Валидация GGUF: `ggml/src/gguf.cpp:727-746` — тип вне `[0, GGML_TYPE_COUNT)` отвергается.
- Лишние тензоры: `src/llama-model-loader.cpp:1403-1409` — `too many tensors` / `wrong number of tensors` (типично для vision-тензоров в одном файле).
- Chat template: `common/chat.cpp:884-913` — при отсутствии шаблона берётся ChatML; `:940-965` — BOS/EOS из вокабуляра.
- JNI: `app/src/main/cpp/snapcore.cpp:151-156` (KV), `:158-167` (`add_bos=false`), `:253-258` (mmap/ngl), `:263-268` (потеря причины ошибки), `:328`, `:433-460` (trim/reserve).
- UI: `Engine.kt:95-113` (`engine load failed`), `ModelStore.kt:43` (только `.gguf`), `MainActivity.kt:38` (`*/*`), `Prefs.kt:68-76`, `Panels.kt:276,283,338` (ctx до 32768, max tokens до 2048).
- Сборка: `app/src/main/cpp/CMakeLists.txt:10-14` — `LLAMA_SRC=external/llama-clover-src`; `app/build.gradle.kts` — только `arm64-v8a`, `GGML_NATIVE=OFF`, без KleidiAI.
- Mainline-различия: `external/llama.cpp/src/llama-arch.cpp:93,106,151,161,171,181`; `external/llama.cpp/ggml/include/ggml.h:435` — `F8_E4M3=43`.

## Что реально ограничивает APK

1. **Только текст.** `mtmd` лежит в `tools/mtmd`, в APK не линкуется; в UI нет выбора изображения/аудио. Любой mmproj бесполезен.
2. **Только CPU.** `gpuLayers=0`; сборка baseline ARMv8-A (`GGML_NATIVE=OFF`, без `-march`/dotprod/i8mm по `ggml-cpu/CMakeLists.txt:206-256`). Кванты работают, но медленнее.
3. **6 GB RAM.** Практически ~3–3.5 GB под процесс; остальное — page cache для mmap (`docs/13-CLOVER.md`: peak RSS 4.26 GB у Clover).
4. **ctx/max_tokens.** Дефолт ctx 4096, max tokens 512; UI позволяет 2048–32768 и до 2048 токенов, но это не отменяет RAM. При переполнении ctx ответ молча обрывается: `nativeNext` возвращает null, а UI показывает частичный текст без ошибки (`snapcore.cpp:526-530`, `AppController.kt:262-266`).
5. **Chat template.** Если в GGUF нет `tokenizer.chat_template`, подставляется ChatML без предупреждения. Плюс `add_bos=false` — для Gemma/Mistral-подобных SentencePiece-моделей возможна деградация (гипотеза, требует теста).
6. **Токенайзеры.** Поддержаны SPM, BPE, WPM, UGM, RWKV, PLaMo2/3 (`include/llama.h:73-81`); это почти никогда не блокер.
7. **Файловая логика.** Импорт принимает `*/*` без проверки; показываются все `.gguf`, включая mmproj и отдельные шарды split-модели. Выбор шарда не первый → ошибка загрузки.

## Лестница тестовых моделей под 6 GB

| Тип | Модель / квант | Размер (≈) | Ожидание |
|---|---|---|---|
| tiny | SmolLM2-135M-Instruct Q4_K_M | ~0.10 GB | дымовой тест UI/шаблона |
| tiny | Qwen2.5-0.5B-Instruct Q4_K_M | 0.40 GB (локально 397 808 192 B) | эталон, уже проверен |
| small | Qwen2.5-1.5B-Instruct Q4_K_M | ~1.0 GB | базовый чат |
| small | Qwen3-1.7B Q4_K_M | ~1.1 GB | thinking-шаблон |
| small | Gemma-2-2B-it Q4_K_M | ~1.7 GB | проверка BOS/шаблона |
| small | Llama-3.2-3B-Instruct Q4_K_M | ~2.0 GB | верх комфорта |
| small | Qwen3-4B Q4_K_M | ~2.5 GB | граница; следить за LMK |
| small | Phi-3-mini-4k-instruct Q4_K_M | ~2.4 GB | альтернативный шаблон |
| MoE | OLMoE-1B-7B-0924-Instruct Q4_K_M | ~4.2 GB | лучший MoE-тест на 6 GB |
| MoE | Qwen1.5-MoE-A2.7B-Chat Q4_K_M | ~9 GB | storage-bound, как Clover |
| MoE | DeepSeek-V2-Lite-Chat Q4_K_M | ~10.4 GB | только демо |
| MoE | Clover-35B-A3B APEX-I-NanoPlus V2.1 | 11.69 GB | уже работает, ~0.6 tok/s |
| ternary | BitNet b1.58 2B4T TQ1_0/TQ2_0 | ~0.6–1.1 GB | **проверить на HF формат** |
| ternary | Falcon3-1.58bit 7B TQ1_0 | ~2–3 GB | **проверить на HF** |
| ternary | TriLM-3B TQ2_0 | ~0.9 GB | **проверить на HF** |

Размеры — приблизительные, по имени/кванту; точные байты смотреть на HF. Ссылки не приводятся намеренно.

## План проверки на устройстве (не выполнять здесь)

```powershell
# 1. Подготовка (host)
adb shell am force-stop dev.snapdragonllm
adb push model.gguf /sdcard/Android/data/dev.snapdragonllm/files/models/
adb push model.gguf /data/local/tmp/sllm/

# 2. CLI-прогон тем же деревом 4838ffb
adb shell "cd /data/local/tmp/sllm && LD_LIBRARY_PATH=. ./llama-completion -m model.gguf -p 'Hello' -n 16 -c 1024 -t 4 -ngl 0"
adb shell "cd /data/local/tmp/sllm && LD_LIBRARY_PATH=. ./llama-bench -m model.gguf -p 128 -n 32 -t 4"

# 3. Приложение: импорт → Load → один короткий запрос
adb logcat -c
adb logcat -s SLLM:V
# искать: "unknown model architecture", "unsupported model architecture",
#         "invalid ggml type", "wrong number of tensors", "failed to create llama context"

# 4. Память
adb shell dumpsys meminfo dev.snapdragonllm
adb shell "cat /proc/$(adb shell pidof dev.snapdragonllm)/status"
```

Для метаданных без запуска: `llama-completion -m file.gguf --verbose` или Python-пакет `gguf` (`GGUFReader`) — вывести `general.architecture`, `general.file_type`, `tokenizer.chat_template`, список tensor types. Порядок прогона: tiny → small → MoE → ternary.

## Риски и неопределённости

- **Ошибки загрузки не диагностируются.** `nativeLoad` возвращает 0, причина удаляется вместе с engine; в UI остаётся `engine load failed`. Это надо исправить в первую очередь, иначе матрица проверяется только по logcat.
- **6 GB и LMK.** Для файлов >4 GB процесс может быть убит во время генерации без исключения; UI покажет просто пропажу ответа.
- **«Arch есть» ≠ «граф работает на CPU».** Экзотические (`qwen4exp`, `kimi-k3`, `glm5-next`, `deepseek41`) могут упасть на отсутствующей CPU-операции или assert; статус `likely` — именно про это.
- **Mainline ≠ APK.** Нельзя ориентироваться на свежие релизы llama.cpp: 6 архитектур и `F8_E4M3` появились после 4838ffb.
- **Clover-патч.** `patches/llama-mainline-clover-bd-load.patch` относится только к `external/llama.cpp`; APK его не применяет. В 4838ffb `clover_moe` работает нативно, worktree чистый.
- **BOS/шаблон.** Отсутствие шаблона и `add_bos=false` — тихие деградации, а не ошибки; их легко принять за «модель плохая».
- **Размеры и ternary-форматы.** Точные имена/кванты BitNet/Falcon3/TriLM нужно сверять на HF; официальный BitNet может быть в I2_S, который этот загрузчик не поймёт.
