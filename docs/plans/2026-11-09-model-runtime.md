# План: Rust-модели и движок — «работают идеально»

Дата: 2026-11-09. Заменяет разрозненные заметки по моделям.
Цель: не «модель загрузилась», а **предсказуемо правильный вывод, честные
ошибки и измеренная производительность** на MI 8 (845, 6 GB) и Pad 2
(8 Gen 3, 16 GB).

## 0. Что значит «идеально»

1. **Корректность.** Тот же prompt + seed + temp=0 даёт тот же текст, что
   `llama-cli` на том же дереве llama.cpp.
2. **Честный preflight.** До загрузки весов известно: формат, квант,
   архитектура, split-шары, LoRA/mmproj, нужная память, поддержан ли квант
   этим билдом.
3. **Никаких «load failed» без причины.** Ошибка имеет код, текст и
   следующий шаг для пользователя.
4. **Память под контролем.** OOM/trim/выгрузка модели не роняют процесс.
5. **Скорость измерена** на устройстве, а не обещана.
6. **Ничего не утекает**: нет сети по умолчанию, нет фоновой загрузки,
   промпты не логируются.

## 1. Крейты и границы

| Крейт | Ответственность | Не делает |
|---|---|---|
| `sllm-gguf` | парсер заголовка/тензоров, split, SHA-256, типы | не грузит веса |
| `sllm-model` | манифест, реестр, группировка base+LoRA+mmproj, capability, fit | не считает токены |
| `sllm-llama-sys` | bindgen + C++ shim + CMake, pinned llama.cpp | не содержит логики приложения |
| `sllm-engine` | load/generate/stream/cancel/trim/stats, sampler, KV, threads | не знает про UI |
| `sllm-app` | HTTP/JNI-мост, WebView/Slint, настройки | не парсит GGUF |

Правило: UI видит только `ModelSummary`, `PreflightReport`, `EngineEvent`.
Никаких `llama_*` указателей в UI-слое.

## 2. Матрица моделей и форматов

| Модель | Арх | Квант | Размер | Особенности | Статус |
|---|---|---|---|---|---|
| Qwen2.5 0.5B | qwen2 | Q4_K_M | 0.37 GB | baseline, 24 слоя | must |
| Ternary-Bonsai 2-4B | qwen35 | Q2_0 | 1.13 GB | ternary, hadamard v2 | must |
| Bonsai Uncensored LoRA | adapter | rank-1 | 2.37 MB | runtime, не merge | must |
| Clover 35B-A3B | clover_moe | Q4_0 | 11.69 GB | MoE 256e/8a, только 4838ffb | must |
| PTQ1_0 / PQ2_0 | — | 143 / 142 | — | не поддержаны 4838ffb | explicit unsupported |
| Split GGUF | any | any | — | `-00001-of-000NN` | must |
| mmproj | clip | — | — | vision, если есть | should |
| Обычный GGUF | llama | Q4/Q5/Q6/Q8 | — | fallback | should |

Правило: неподдержанный квант — это **не** «engine load failed», а
`E_QUANT_UNSUPPORTED` с текстом «этот билд не содержит PTQ1_0; используйте
Q2_0 или mainline-билд».

## 3. Жизненный цикл модели

```
Discovered → Preflighted → Compatible → Loading → Ready
                │              │           │
                │              │           └→ E_LOAD_* (OOM, corrupt, mmap)
                │              └→ E_INCOMPATIBLE (quant/arch/split)
                └→ E_FORMAT (magic/version/truncated)
Ready → Generating → Ready
Ready → Trimming → Ready
Ready → Unloading → Discovered
```

Инварианты:

- одновременно загружена **одна** тяжёлая модель;
- `Ready` невозможен без успешного `Preflighted`;
- `Generating` можно отменить в любой момент, состояние возвращается в
  `Ready`;
- `Trim(Complete)` освобождает веса, но оставляет метаданные и путь;
- после `Trim` повторный `load` не требует повторного preflight.

## 4. Preflight (без загрузки весов)

Проверять в этом порядке, с быстрым выходом:

1. file exists, size > 0, readable;
2. magic `GGUF`, version 2/3, header bounds;
3. `general.architecture`, `general.type` (`model` / `adapter` / `mmproj`);
4. `general.file_type` и реальные типы тензоров;
5. архитектура известна этому билду;
6. split: `split.count/no`, все шары на месте, размеры совпадают;
7. tokenizer: `tokenizer.ggml.model/tokens/merges`, `chat_template`;
8. tensor count/offsets не выходят за файл; нет NaN-размеров;
9. нужный квант есть в `sllm-llama-sys` (`sllm_supports_type`);
10. companion-файлы: LoRA (`general.type=adapter`), mmproj (`clip`);
11. оценка памяти: weights + KV + recurrent + compute + LoRA + mmproj;
12. SHA-256 (опционально, по запросу; не на каждом старте).

Выход: `PreflightReport { ok, code, message, detail, warnings, arch, quant,
n_layer, n_ctx_train, n_embd, n_head_kv, estimate_mb, capabilities }`.

Оценки:

```
weights   = file_bytes (mmap/PSS)
KV        = n_ctx × n_full_attn × n_head_kv × (k_len + v_len) × bytes(kv)
RS        = recurrent state для SSM/Mamba-слоёв
compute   = batch × n_embd × bytes + logits
adapters  = сумма размеров LoRA
mmproj    = размер + буфер изображения
reserve   = 1.5 GB (6 GB) / 2.5 GB (16 GB) / 4 GB (24 GB)
```

Для Clover: KV q8_0 ctx4096 ≈ 45 MB, RS ≈ 63 MB, compute ≈ 127 MB.
Для Bonsai: KV q8_0 ctx4096 ≈ 68 MB, RS ≈ 50 MB.

## 5. Engine API

```rust
pub struct Engine { /* opaque */ }

impl Engine {
    pub fn load(pack: &ModelPack, cfg: &EngineConfig) -> Result<Self, SllmError>;
    pub fn generate(&mut self, req: &GenerateRequest,
                    sink: &mut dyn FnMut(EngineEvent)) -> Result<GenStats, SllmError>;
    pub fn cancel(&self);
    pub fn trim(&mut self, level: TrimLevel);
    pub fn stats(&self) -> EngineStats;
    pub fn unload(self);
}

pub enum EngineEvent {
    Started { prompt_tokens: usize },
    Token { text: String, id: u32 },
    Done { stats: GenStats },
    Error { code: ErrorCode, message: String },
}
```

Требования:

- `generate` не блокирует UI-поток: работает на выделенном потоке, события
  идут через канал/JNI-callback.
- `cancel` выставляет atomic и будит поток; следующий токен не генерируется.
- `GenerateRequest`: messages, sampler, max_tokens, stop, grammar (позже),
  `n_batch`, `n_threads`.
- `GenStats`: prompt_tokens, generated_tokens, ttft_ms, prefill_ms,
  decode_ms, tok_per_s, peak_rss_mb, backend.

## 6. Токенизация и chat templates

- Использовать tokenizer llama.cpp, не писать свой BPE.
- `common_chat_templates_init(model, override)` и
  `common_chat_templates_apply` для Qwen/Bonsai/Clover.
- Golden-тесты: для каждого шаблона фиксированный массив сообщений →
  ожидаемая строка prompt (сравнение с `llama-cli --verbose-prompt`).
- Проверять BOS/EOS, `add_bos`, `add_eos`, роль system, пустой system,
  multi-turn, tool-call (позже).
- Clover: шаблон и special tokens проверить на 4838ffb отдельно.

## 7. Sampling

- Цепочка: `top_k` → `top_p` → `min_p` → `temperature` → `repeat_penalty`
  → `dist` с seed.
- `temp=0` — argmax, для golden-тестов.
- Stop strings: обрезать до сохранения в историю, поддержка нескольких.
- Seed фиксируется в `GenerateRequest`; в stats возвращается.
- Не менять sampler между токенами; состояние создаётся на запрос.

## 8. LoRA и mmproj

LoRA:

- `llama_adapter_lora_init` / `_from_file_ptr` (SAF);
- `llama_set_adapters_lora(ctx, adapters, n, scales)`;
- проверка: архитектура совпадает, rank/тензоры совместимы, файл — adapter;
- **не merge** в ternary/Q2_0;
- тест: Bonsai Q2_0 + Uncensored LoRA даёт другой ответ, +4.2 MB RSS,
  дельта decode в пределах шума;
- UI: бейдж LoRA, переключатель, scale, предупреждение о несовместимости.

mmproj:

- `mtmd_init_from_file`, `mtmd_default_marker`, `mtmd_tokenize`,
  `mtmd_helper_eval_chunks`;
- capability `vision` включается только при наличии mmproj и поддержке
  маркера;
- изображение: downscale до 1024–1280 px, JPEG/PNG, без EXIF-сюрпризов;
- UI: attach-image активен только у vision-модели, иначе disabled + причина.

## 9. Память, потоки, backends

- Persistent threadpool: `ggml_threadpool_params_default(n)` +
  `llama_attach_threadpool`, не создавать на запрос.
- Потоки: 845 — 4 big; 8 Gen 3 — 4–6; 8 Elite — 6–8; маска по
  `cpuinfo_max_freq`, не хардкод.
- KV: q8_0 по умолчанию; q4_0 — только для длинного ctx/6 GB, с quality gate.
- mmap: `MMAP`, не `MLOCK`; `MADV_WILLNEED` для preload,
  `MADV_DONTNEED` при trim.
- `onTrimMemory`: MODERATE — кэш изображений; RUNNING_LOW — KV/RS;
  COMPLETE — выгрузить веса, оставить метаданные.
- Backends: CPU baseline для 845; v82/v86/v90 для новых; OpenCL opt-in;
  Vulkan/Hexagon — после CPU-базлайна; при ошибке — fallback CPU + лог.

## 10. Тесты и quality gates

### Unit / integration

- GGUF: корректные заголовки Qwen/Bonsai/Clover/LoRA; обрезанный файл;
  неверный magic; гигантские размеры; split 1/3, 2/3, 3/3; fuzz
  (`cargo-fuzz`, parser не паникует).
- Preflight: каждый `ErrorCode` достижим и имеет сообщение; unsupported
  quant распознаётся до загрузки.
- Tokenizer/template: golden-строки для 3 моделей.
- Sampling: temp=0 детерминирован; seed воспроизводим.
- LoRA: detection, совместимость, scale.
- Engine: load/unload 100 циклов без роста RSS; cancel на 1-м токене;
  trim(Complete); повторный load после trim.

### Device gates

| Модель | MI 8 (845, 6 GB) | Pad 2 (8 Gen 3, 16 GB) |
|---|---|---|
| Qwen 0.5B Q4_K_M | ≥ 25 tok/s (факт 25–26) | ≥ 40 tok/s |
| Bonsai Q2_0 | ≥ 2.5 tok/s (факт 2.55) | ≥ 8 tok/s |
| Bonsai + LoRA | без падения, +≤10 MB | то же |
| Clover Q4_0 | ≥ 0.6 tok/s (факт 0.60–0.67) | ≥ 2 tok/s |

Дополнительно: TTFT, prefill, peak PSS, 30-минутный soak, троттлинг
(≤ 15% падения), отсутствие OOM, 16 KB page size.

### Golden output

- Qwen 0.5B, prompt из фикстуры, temp=0, seed=42, max_tokens=64 →
  SHA-256 текста совпадает с `llama-cli` на том же дереве.
- Bonsai/Clover — smoke + ручная проверка связности, без побайтового
  сравнения (MoE/ternary могут зависеть от порядка reduction).

## 11. Ошибки и UX

| Код | Когда | Что видит пользователь |
|---|---|---|
| `E_FORMAT` | не GGUF/битый | «Файл повреждён или это не GGUF» |
| `E_QUANT_UNSUPPORTED` | PTQ1_0/PQ2_0 | «Этот билд не поддерживает квант; выберите Q2_0» |
| `E_ARCH_UNSUPPORTED` | неизвестная арх | «Архитектура не поддержана этим движком» |
| `E_SPLIT_MISSING` | нет шара | «Не хватает файла 2 из 3» |
| `E_RAM_LOW` | estimate > budget | «Нужно 12.7 GB, доступно 4.8 GB» |
| `E_OOM` | mmap/alloc | «Не хватило памяти; выгрузите другую модель» |
| `E_IO` | файл исчез | «Модель перемещена или удалена» |
| `E_CANCELED` | cancel | не ошибка, «Остановлено» |

Ошибки не показываются как тост на пол-экрана: строка в карточке модели +
«Details» с кодом и советом.

## 12. Менеджер моделей

- Источники: локальная папка, SAF-импорт, позже — URL с SHA-256.
- Импорт: `.part` → SHA-256 → atomic rename; прогресс; отмена; проверка
  свободного места до копирования.
- Карточка: имя, arch, quant, размер, fit, capability, статус, активная.
- Действия: Set active, Details, Delete (с подтверждением), Open folder.
- Split: показывать как одну модель, а не N файлов.
- LoRA/mmproj: показывать как companion, не как отдельные модели.
- Никаких автозагрузок из сети без явного действия.

## 13. Этапы

### M0 — preflight и реестр (сейчас)

- `sllm-model`: манифест, группировка, dedupe, fit.
- `sllm-gguf`: fuzz-тесты, split, SHA-256.
- UI Models читает реестр (уже есть).
- Выход: 3 модели + LoRA корректно описаны, unsupported quant — честно.

### M1 — sys + Qwen

- `sllm-llama-sys` (bindgen + shim + CMake), baseline armv8-a.
- load Qwen 0.5B, streaming, cancel, stats.
- Golden output vs llama-cli.
- Выход: Qwen на MI 8 ≥ 25 tok/s, cold TTFT < 1.5 s.

### M2 — Bonsai + LoRA + KV

- Q2_0, hadamard, LoRA runtime, KV q8_0/q4_0, persistent pool.
- Выход: Bonsai ≥ 2.5 tok/s на MI 8, LoRA без merge и без утечек.

### M3 — Clover

- 4838ffb, `llama_process`/`batch_ext`, mmap/readahead, ctx 2048/4096.
- Выход: Clover ≥ 0.6 tok/s на MI 8; на Pad 2 — замер.

### M4 — Pad 2 / v86 / backends

- CPU v86, 6 threads, OpenCL opt-in, thermal soak.
- Выход: таблица backend × model × device.

### M5 — mmproj и полировка

- vision, split GGUF, импорт SAF, trim/OOM, 30-мин soak.
- Выход: все заявленные сценарии проходят на обоих устройствах.

## 14. Риски

- Clover 4838ffb bitrot → vendored tarball + SHA + CI на двух деревьях.
- WebView UI добавит PSS → бюджет и fallback на Slint.
- Термальный троттлинг 845 → 30-мин soak, ограничение потоков.
- LoRA на ternary может быть чувствителен к порядку → тест на реальном
  адаптере, не обещать «как в fp16».
- Split GGUF и SAF: URI не даёт `Range` → копирование целиком, проверка
  места.
- «Идеально» не значит «быстро на всех»: честные таблицы и fallback.
