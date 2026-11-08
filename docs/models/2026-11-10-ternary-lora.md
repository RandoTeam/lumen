# Ternary-Bonsai-2-4B: Q2_0, PTQ1_0 и Uncensored LoRA

Дата: 2026-11-10. Дерево `external/llama-clover-src` (4838ffb), файлы `models/ternary/`.
Метод: разбор GGUF, SHA-256, расчёт по байтам, код llama.cpp, `sllm-cli check`, логи.
Не измеренное — «проверить».

## 1. Точные метаданные

Q2_0 и PTQ1_0 — один граф: 426 тензоров, 50 KV, все ключи совпадают, кроме `general.file_type`.

| Поле | Q2_0 | PTQ1_0 |
|---|---|---|
| Размер | 1 212 103 520 B (1.129 GiB) | 949 549 920 B (0.884 GiB) |
| SHA-256 | `64a5183b…9433d4` (= HF) | `56711263…994d60` (= HF) |
| `file_type` | 41 (`LLAMA_FTYPE_MOSTLY_Q2_0`) | **143**, нет в enum |
| Тензоры | 201×Q2_0 + 225×F32 | 201×type 143 + 225×F32 |
| Параметры | 4 205 751 296 (4.206B) | те же |
| Данные / заголовок | 1 201.1 MB / 11.04 MB | 938.5 MB / 11.04 MB |
| Бит на вес | 2.25 = 18 B / 64 | 1.75 = 28 B / 128 (вывод из кратности) |
| Блоки / ctx / embd / ff | 32 / 262 144 / 2560 / 9216 | те же |
| Heads / KV / dim | 16 / 4 / 256 | те же |
| RoPE / SSM | dim 64, base 1e7, `[11,11,10,0]`; conv 4, state 128, groups 16, inner 4096 | те же |
| Full attention | interval 4 → 8 слоёв (3,7,…,31), 24 GDN | те же |
| Hadamard | v2, block 512, `normalized-sylvester-walsh-hadamard`, axis `input-last-dimension`, `sign_mode=explicit`, `tied_output=true`, `gdn_v_grouped=true` | те же |
| Таблицы | 200 folded + `token_embd` inverse; `sign_widths=[2560,4096,9216]`, `sign_values` 15 872 | те же |
| Токенайзер | gpt2/pre `qwen35`, 248 320; eos 248046 `im_end`, bos/pad 248044, `add_bos=false` | те же |
| Template | 7 756 симв., ChatML, `<think>`, tools, vision-макросы, `enable_thinking` | тот же |
| Split | нет | нет |

Раскладка сходится: 201 = `token_embd` + 96 ffn + 72 GDN-сайта + 32 full-attn;
225×F32 = 64 нормы + 144 SSM + 16 q/k-norm + `output_norm`. `output.weight` нет — голова
связана с эмбеддингом, `token_embd` хранится повёрнутым.

Коды Q2_0 (выборка): 34.1% `-1`, 31.8% `0`, 34.1% `+1`, кода `+2` нет. Веса действительно
тернарные, `d` блока (fp16, `max|w|`) — их масштаб. Раскладка PTQ1_0 в дереве отсутствует:
«28 B / 128» — арифметика.

### LoRA

| Поле | Значение |
|---|---|
| Файл | 2 368 992 B (2.26 MiB): данные 2 359 296, заголовок 9 696 |
| SHA-256 | `605e9dfb…138a9` (= HF) |
| KV (14) | `general.type=adapter`, `adapter.type=lora`, `adapter.lora_alpha=1.0`, arch `qwen35` |
| Тензоры | 128×F32, 589 824 параметра |
| Сайты | 64 = 32 `ffn_down` + 24 `ssm_out` + 8 `attn_output`, все 32 блока |
| Форма | A `[in,1]`, B `[1,out]`, rank 1; `‖B‖=1.000000` для всех сайтов, `‖A‖` ≈0.69–0.88 |
| База | `base_gguf=…PTQ1_0.gguf`, `base_sha256=56711263…` |
| Прочее | `direction_row=31`, `target_layers=all`, `target_embedding=no`, `sites=64`, `basis=unfolded input basis` |

SHA локального PTQ1_0 равен `base_sha256`: адаптер обучен на нём, не на Q2_0 (автор Q2_0/PQ2_0
не запускал).

## 2. Почему PTQ1_0/PQ2_0 не работают в 4838ffb

`GGML_TYPE_COUNT = 43` (0…42), `PQ2_0 = 142`, `PTQ1_0 = 143` отсутствуют.
`gguf_init_from_file()` создаёт тензор с типом из файла, а `ggml_new_tensor_impl()` требует
`type < GGML_TYPE_COUNT`: прямое открытие — **abort процесса**, не исключение. Поэтому
preflight обязателен; `sllm-cli` отдаёт `E_QUANT_UNSUPPORTED` до загрузки.

Нужна ветка `prism` в [PrismML-Eng/llama.cpp] (проверено на `prism-b10777-e3de715`;
`prism-b10709` падает с `unsupported prism.hadamard.version: 2`). Upstream с b11556 читает
только Q2_0. Патч нашего форка — блоки, dequant/quant, `vec_dot` x86/ARM, repack/quantize,
`GGML_TYPE_COUNT=144`; 0.26 GB экономии того не стоит. Пользователю: «PTQ1_0 (0.95 GB) не
читается этим билдом; используйте Q2_0 (1.21 GB)».

## 3. LoRA: рантайм, база, merge

`build_lora_mm()` считает базовый матмул по повёрнутому входу (`W_folded·H·D·x`) и отдельно
`B·A·x` по исходному, затем складывает. Веса не меняются. Для hadamard-весов это
принципиально: дельта живёт в primal-базисе, и метаданные подтверждают пересчёт A в
«unfolded input basis». `token_embd` запрещён явно: таблица хранит повёрнутые строки.

Эффективный множитель `scale = adapter_scale·alpha/rank = 1.0`. Тонкость: файл содержит
`adapter.lora_alpha`, а 4838ffb читает `adapter.lora.alpha`; ключа нет → `alpha=0` →
используется просто `adapter_scale`. Для rank=1/alpha=1 числа совпадают, для другого
адаптера была бы ошибка. UI должен показывать эффективный scale.

Проверяются arch, наличие тензора и формы; хеш базы **не** проверяется, поэтому адаптер
поверх Q2_0 грузится, хотя объявленная база — PTQ1_0. Карточка Prism: все паки хранят одни
и те же тернарные веса, dequant Q2_0 точен, перенос корректен. Но Q2_0 автор не тестировал,
утечка ~1e-4 измерена только на PTQ1_0 — это warning.

Память/скорость: +2.36 MB и +589 824 MAC на токен = **0.014%** от 4.2B MAC. Логи: база
2.39–2.55 tok/s, с LoRA 2.30–2.41; разные прогоны, чистого A/B нет — «проверить».
`llama_set_adapters_lora` меняет адаптер без перезагрузки и сброса KV (по коду).

Merge бессмысленен, это посчитано по файлу. При записи `W+Δ` в Q2_0 квантователь берёт
`d'=max|W+Δ|` на блок 64 и те же 2-битные коды: ни один код не меняется (0.00%), RMSE
4.16–4.33% против 1.9–2.0% у самой дельты (base RMS 0.0086–0.0110, Δ RMS 0.00017–0.00023).
Дельта меньше половины шага, поэтому merge превращается в сдвиг fp16-масштаба блока — в
блочный gain, а не в направление; ошибка re-quant вдвое больше сигнала. Плюс
`llama-export-lora` пишет F16 (≈8.4 GB) и не знает про hadamard. Вывод: только runtime.

## 4. Uncensored: что реально делает адаптер

Это abliteration, не «снятие цензуры». Направление `r` — разность средних residual stream на
последнем токене (416 вредных vs 416 безвредных), строка 31 (Cohen's d 10.6, AUROC 0.9994),
с проекцией на безвредное среднее. Дельта на каждом сайте: `Δ = -r·(a_site·x)` — из выхода
слоя вычитается компонента вдоль `r`. `B=-r` общий, `A` свой.

Заявлено автором (не воспроизведено): отказы 99%→0% (104 held-out вредных), безвредные
1%→0%, MMLU 68.7→69.0 ±2.7 (n=300, thinking off), утечка ~1e-4. На Q2_0 не
измерялось; лога с провокационным промптом в репозитории нет — **проверить**.

Проверка: 104+104 промпта AtomicChat, greedy, thinking off, общий system prompt/seed; отказы
по первым фразам, пустые/деградировавшие — damage; отдельно безвредный набор. MMLU на
телефоне не гонять, хватит 22-промптного gate.

Откат: `scale=0` или тумблер off без перезагрузки; удалить файл. Базовый GGUF не меняется,
SHA остаётся `64a5183b…`, откат полный.

Юридически: «удаляет обученное направление отказа, research-grade; не гарантирует
безопасность, корректность или законность; ответственность на пользователе». Не писать
«снимает цензуру»/«без ограничений». Нужны 18+, opt-in, скрытие в детских профилях,
Apache-2.0 + NOTICE (Prism ML, Qwen/Alibaba, AtomicChat); по умолчанию выключен.

## 5. План тестов

| # | Проверка | Условие | Ожидание |
|---|---|---|---|
| T1 | Preflight | Q2_0 / PTQ1_0 / LoRA | `ok=true` / `E_QUANT_UNSUPPORTED` (143) / `E_LORA_NOT_MODEL` |
| T2 | Загрузка + детерминизм | 3× `--temp 0 -s 42 -n 96` | `load time` 2.1–2.7 s; одинаковый текст/SHA |
| T3 | LoRA on/off/0 | `:1` и `:0` | scale 0 = T2 побайтно; scale 1 меняет ответ |
| T4 | Scale sweep | 0, 0.5, 1, 1.5, 2 | нет «here here here»; >2 клампить |
| T5 | Память | VmRSS/VmHWM из `run_model.sh` | пик 1.3–1.5 GiB; +LoRA ≤10 MB; 50 toggle без роста |
| T6 | Скорость | `llama-bench -t 4 -p 128 -n 32 -r 3` | pp≈3.2, tg≈2.4 t/s; LoRA ≤5% |
| T7 | Качество | 22 промпта, temp 0 | base≈LoRA, 0 пустых |
| T8 | Отказы | 104+104, greedy | base 99% → LoRA 0%; на Q2_0 проверить |
| T9 | Ошибки | чужой arch, обрезанный файл | «arch mismatch», `E_GGUF_TRUNCATED` |

## 6. Команды и ожидаемые поля

```powershell
$env:CARGO_TARGET_DIR="target/agent-ternary"
cargo run -q -p sllm-cli -- check models/ternary/Ternary-Bonsai-2-4B-Q2_0.gguf
# ok=true code=None arch=Some("qwen35") quant=Some("Q2_0") est=1955 MB
cargo run -q -p sllm-cli -- check models/ternary/Ternary-Bonsai-2-4B-PTQ1_0.gguf
# ok=false code=Some(QuantUnsupported) quant=Some("GGUF") est=1705 MB
cargo run -q -p sllm-cli -- check models/ternary/Ternary-Bonsai-2-4B-Uncensored-LoRA.gguf
# ok=false code=Some(LoraNotModel) est=530 MB
python models\ternary\gguf_inspect.py models\ternary\Ternary-Bonsai-2-4B-Q2_0.gguf
```

На устройстве (`--lora-scaled` режет по `:`, путь к адаптеру — относительный):

```sh
./llama-completion -m base.gguf -t 4 -c 1024 -n 96 --temp 0 -s 42 --no-warmup \
  --no-display-prompt -p 'Hello! Reply in one short sentence.'
./llama-completion -m base.gguf --lora-scaled lora.gguf:1 -t 4 -c 1024 -n 96 \
  --temp 0 -s 42 --no-warmup --no-display-prompt -p 'Hello! Reply in one short sentence.'
./llama-bench -m base.gguf -t 4 -p 128 -n 32 -r 3 -o json
```

Поля: `load time`, `prompt eval time`, `eval time`, `graphs reused`; JSON llama-bench —
`n_prompt`, `n_gen`, `avg_ts`, `stddev_ts`; память — `rss_kb`, `hwm_kb`, `avail_kb`.
Chat: `--jinja -sys "You are a helpful assistant" --reasoning-budget 0` (или
`--chat-template-kwargs '{"enable_thinking":false}'`), стоп `<|im_end|>` (248046).

Измерено: llama-bench pp128 **3.22 ± 0.13 t/s**, tg32 **2.41 ± 0.00**. Логи: decode
2.39–2.55, prefill 2.9–3.7 (промпт 8 токенов, шумно). Арифметика: 512 токенов ≈ 160 с,
1900 ≈ 10 мин. Долгий prefill — главный UX-риск; 262k контекста на телефоне нереалистичны,
2048–4096 и KV q8_0 (34/68 MiB).

## 7. Registry / engine / UI

**Registry.** Классифицировать по `general.type`; адаптер — companion, не модель. Квант — по
гистограмме, не по имени. Поля: `is_ternary`, `hadamard_version`, `lora_capable`, `thinking`,
`tools`, `vision=false`. Статус `blocked` с кодом для неподдержанного кванта. В `quant_of()`
добавить 142/143 (иначе UI показывает «GGUF»). `estimate_mb()`: KV по `full_attention_interval`
(8 слоёв, не 32), RS ≈ 48 MiB, адаптеру KV не считать (сейчас 530 MB для 2.4 MB). `scan_dir`
не должен показывать PTQ1_0 как модель.

**Engine.** `load` → `lora_init` (`…_from_file_ptr` для SAF) → `set_adapters_lora`; не мержить;
адаптер освобождать до модели. `scale` 0…2, шаг 0.05, default 1.0, `0` = off. `base_sha256`:
несовпадение — warning `E_ADAPTER_BASE_MISMATCH`. Sampling из `general.sampling.*`
(1.0/0.95/20; 0.7/0.8/20 без thinking), presence 1.0/1.5 в конфиге. `max_tokens` 8192/512,
иначе обрыв в `<think>`; `ctx` клампить к 2048–4096.

**UI.** Бейджи `TERNARY Q2_0 · 2.25 bpw`, `4.21B`, `Hadamard v2`, `ctx 262k (device 2–4k)`,
`+ LoRA 2.4 MB`. Предупреждения: «PTQ1_0/PQ2_0 не поддержаны», «адаптер обучен на PTQ1_0,
на Q2_0 не тестировался», «thinking ≥8192 токенов», «prefill ~3.2 t/s». Слайдер 0…2,
«0 = выключено»; не трактовать 100 как проценты (`scale100.log` → «here here here…», команда
не сохранена — **проверить**). 18+, скрытие в детских профилях, «+2.4 MB RAM», «откат
мгновенный».

## 8. Факт / не проверено

**Факт:** размеры, SHA, KV, типы, 2.25/1.75 bpw, 4.206B, hadamard v2, 200 folded + 1 inverse,
LoRA 589 824 параметра/rank 1/общий B, `base_sha256` = SHA PTQ1_0, отсутствие 142/143 и
abort, pp/tg llama-bench, деградация `scale100.log`.

**Не проверено:** RSS/PSS и thermal; чистый A/B с LoRA; качество/отказы на Q2_0;
провокационный промпт (лога нет); раскладка PTQ1_0; `--jinja`; 30-мин soak;
Google Play.

## 9. Пять выводов для реализации

1. **Q2_0 — единственный путь.** PTQ1_0/PQ2_0 требуют PrismML-форка; в 4838ffb это abort,
   поэтому preflight обязан блокировать до загрузки и предлагать Q2_0.
2. **LoRA — только runtime.** Rank-1, 2.4 MB, +0.014% вычислений. Merge в Q2_0 не меняет ни
   одного кода и даёт ошибку 4.2–4.3% против 1.9–2.0% у дельты; F16-merge — 8.4 GB. Кнопку
   «merge» не делать.
3. **Адаптер обучен на PTQ1_0, не на Q2_0.** Хеш совпадает с PTQ1_0; Q2_0 автор не тестировал.
   Загрузка пройдёт (arch/shape), но нужен warning и собственный eval.
4. **«Uncensored» — abliteration одного направления**, не снятие цензуры. Нужны scale 0…2 с
   клампом, откат в один клик, 18+, выключено по умолчанию, честная формулировка.
5. **2.4 tok/s decode и 3.2 tok/s prefill** — это короткие диалоги, не RAG и не 262k.
   Thinking по умолчанию выключать, `max_tokens` 512, иначе обрыв в `<think>`.
