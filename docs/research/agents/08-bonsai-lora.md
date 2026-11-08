# Отчёт 08. Ternary-Bonsai-2-4B и Uncensored LoRA (полная версия)

Источник: параллельное исследование SnapdragonLLM, 2026-11-08.
Дерево: `external/llama-clover-src` (4838ffb). Устройство: MI 8, SD845, 6 GB.

## TL;DR

1. **Это не «2.4B», а 4.21B**: репозиторий называется «Bonsai **2**, **4B**» — база Qwen3.5-4B. Архитектура `qwen35` (dense hybrid: 32 слоя, 8 full-attention + 24 Gated DeltaNet), **не MoE** (`n_expert = 0`). Файл Q2_0 — 1.21 GB.
2. **В дереве 4838ffb грузится только `Q2_0`.** `PTQ1_0` (type **143**) и `PQ2_0` (type **142**) — приватные типы PrismML; в локальном `ggml.h` `GGML_TYPE_COUNT = 43`, поэтому загрузчик отвергает файл на этапе чтения tensor info. Патч «на 20 строк» тут не поможет: нужны ядра для group-128 форматов. Рекомендация — брать Q2_0.
3. **LoRA работает поверх Q2_0, рантаймом, без merge.** Адаптер rank-1, F32, 64 сайта, 2.37 MB. Проверено фактическим запуском: база на провокационном промпте выдаёт только дисклеймер, с `--lora-scaled …:1` — начинает отвечать по существу. Скорость не изменилась в пределах шума.
4. **Hadamard v2 поддерживается** именно для `qwen35`: ключи `prism.hadamard.*` читаются, arch в списке «verified», block_size 512 делит 2560/4096/9216. Это не гипотеза — модель загрузилась и сгенерировала текст бинарником из этого дерева.
5. **Оценка на MI 8 [ГИПОТЕЗА]**: 3–5 tok/s decode, 5–8 tok/s prefill, ~1.3 GiB RSS при ctx 4096 (измерено на десктопе: model 1145 MiB + KV 68 MiB q8_0 + RS 50 MiB + compute 22 MiB).
6. Интеграция: 2 новые JNI-функции (`nativeLoraLoad`, `nativeLoraSet`), фильтр адаптеров в `ModelStore`, хранение `lora_path/scale/enabled` в `Prefs`, toggle + слайдер в Models. Merge в ternary-файл невозможен — только runtime.

## 1. Файлы репозиториев (HF API, точные размеры)

`prism-ml/Ternary-Bonsai-2-4B-gguf` (Apache-2.0, base Qwen3.5-4B):

| Файл | Размер | SHA-256 (LFS) | Годится? |
|---|---:|---|---|
| `Ternary-Bonsai-2-4B-Q2_0.gguf` | 1 212 103 520 | `64a5183b…9433d4` | **да, проверено** |
| `Ternary-Bonsai-2-4B-PTQ1_0.gguf` | 949 549 920 | `56711263…994d60` | нет (type 143) |
| `Ternary-Bonsai-2-4B-PQ2_0.gguf` | 1 146 465 120 | `d406b1e9…30409` | нет (type 142) |
| `Ternary-Bonsai-2-4B-PQ2_0-MTP-Q8_0.gguf` | 1 274 642 176 | `0acf517d…6c0d` | нет (PQ2_0 + MTP) |
| `Ternary-Bonsai-2-4B-DFlash2-BF16.gguf` | 1 575 278 208 | `4402be31…08d8` | нет (drafter) |
| `Ternary-Bonsai-2-4B-mmproj-Q8_0.gguf` | 366 894 624 | `db939d5f…a55d` | vision, не нужен |
| `Ternary-Bonsai-2-4B-mmproj-BF16.gguf` | 675 569 184 | `e039156b…448f` | vision, не нужен |
| `README.md` / `USAGE.md` / `LICENSE` / `NOTICE.txt` / `SHA256SUMS` | 10 158 / 1 237 / 10 174 / 599 / 707 | — | — |

`AtomicChat/Ternary-Bonsai-2-4B-Uncensored-LoRA-GGUF` (Apache-2.0, `base_model_relation: adapter`):

| Файл | Размер | SHA-256 (скачан и сверен) |
|---|---:|---|
| `Ternary-Bonsai-2-4B-Uncensored-LoRA.gguf` | 2 368 992 | `605e9dfb…38a9` (совпал с LFS oid) |
| `README.md` | 9 225 | — |

`safetensors`/PEFT в репозитории **нет** — только GGUF-адаптер.

## 2. Архитектура и типы (из заголовка GGUF)

Q2_0, GGUF v3, 426 тензоров, 50 KV. Ключевое:

```
general.architecture = qwen35       general.type = model
block_count 32   embedding 2560    feed_forward 9216
attention.head_count 16   head_count_kv 4
attention.key_length 256  value_length 256
rope.dimension_count 64, freq_base 1e7, dimension_sections [11,11,10,0]
full_attention_interval 4           # 8 full-attn слоёв: 3,7,11,15,19,23,27,31
ssm.conv_kernel 4, state_size 128, group_count 16,
ssm.inner_size 4096, time_step_rank 32
prism.hadamard.version 2, block_size 512,
transform normalized-sylvester-walsh-hadamard, axis input-last-dimension,
sign_mode explicit, tied_output true, gdn_v_grouped true,
weight_names[200], inverse_weight_names["token_embd.weight"],
sign_widths [2560,4096,9216], sign_values[15872]
tokenizer.ggml.model gpt2, pre qwen35, tokens 248320, chat_template 7756 симв.
```

Тензорная гистограмма: **225 × F32 (19.6 MB)** + **201 × Q2_0 (1181.5 MB)**, данные 1201 MB, файл 1212 MB. Никаких TQ1_0/TQ2_0 в этой модели нет; Q2_0 — 2.25 бит/вес (блок 64 элемента, 18 байт: fp16 scale + 16 байт 2-битных слотов). F32 — норм-веса, `ssm_alpha/beta/conv1d/dt/a`, `attn_*_norm`.

**Поддержка в 4838ffb:**
- `GGML_TYPE_TQ1_0=34`, `TQ2_0=35`, `Q2_0=42`, `GGML_TYPE_COUNT=43`. `PQ2_0=142`/`PTQ1_0=143` отсутствуют полностью (проверено `ggml.h`, `ggml.c`, `ggml-common.h`, `ggml-cpu/*`).
- `general.file_type` в Q2_0-файле = 41 (внутренняя нумерация Prism), у PTQ1/PQ2 = 143/141. Это не мешает: ftype только для логов.
- Hadamard: `llama-model.cpp:2073-2400` читает все `prism.hadamard.*`, принимает **version 1 и 2**, а `qwen35` есть в списке «verified» (вместе с llama/qwen3/qwen3moe/qwen35moe/qwen3next). Все 200 folded-весов попадают в белый список kinds (`attn_qkv/attn_gate/attn_output/ffn_*/ssm_out`), `token_embd` идёт как inverse-lookup.
- Q2_0 имеет NEON-ядро `ggml_vec_dot_q2_0_q8_0` в `ggml-cpu/arch/arm/quants.c` (vqtbl + vdotq), repack-пути для Q2_0 нет — будет лог «cannot be used with preferred buffer type CPU_REPACK» и обычный CPU-путь.

**Проверка загрузки (десктоп, бинарник из `external/llama-clover`, b1-4838ffb0):**

```
ftype : Q2_0
[ Prompt: 20.1 t/s | Generation: 12.3 t/s ]   # 4 потока, ctx 512
```
Модель грузится, текст осмысленный, hadamard применяется. Блокеров нет.

**Если всё же нужен PTQ1_0/PQ2_0:** это не «минимальный патч». Придётся добавить `GGML_TYPE_PQ2_0/PTQ1_0` (142/143) в enum и traits, `block_pq2_0/ptq1_0`, dequant/quant/vec_dot для x86/ARM, ветки в repack и в `llama-quantize`. Плюс `GGML_TYPE_COUNT` → 144, а это ломает проверку «тип < COUNT» в старых сборках. Экономия 0.06–0.26 GB не стоит работы; **берите Q2_0**.

**Скрипт разбора заголовка** (качает только первые 16 MB):

```python
import struct, sys, collections
T={0:"F32",1:"F16",8:"Q8_0",10:"Q2_K",12:"Q4_K",14:"Q6_K",30:"BF16",
   34:"TQ1_0",35:"TQ2_0",39:"MXFP4",42:"Q2_0",142:"PQ2_0",143:"PTQ1_0"}
S={0:4,1:2,8:34,10:84,12:144,14:210,30:2,34:54,35:66,39:17,42:18,142:1,143:1}
B={8:32,10:256,12:256,14:256,34:256,35:256,39:32,42:64}
class R:
    def __init__(s,b): s.b=b; s.o=4
    def u32(s): v=struct.unpack_from("<I",s.b,s.o)[0]; s.o+=4; return v
    def u64(s): v=struct.unpack_from("<Q",s.b,s.o)[0]; s.o+=8; return v
    def st(s):
        n=s.u64(); v=s.b[s.o:s.o+n].decode("utf8","replace"); s.o+=n; return v
    def val(s,t):
        if t==8: return s.st()
        if t==9:
            et=s.u32(); n=s.u64()
            if et==8:
                for _ in range(n): s.st()
            else: s.o+= {0:1,1:1,2:2,3:2,4:4,5:4,6:4,7:1,10:8,11:8,12:8}[et]*n
            return None
        sz={0:1,1:1,2:2,3:2,4:4,5:4,6:4,7:1,10:8,11:8,12:8}[t]
        s.o+=sz
r=R(open(sys.argv[1],"rb").read()); ver=r.u32(); nt=r.u64(); nkv=r.u64()
for _ in range(nkv):
    k=r.st(); t=r.u32(); r.val(t)
    if k.endswith(("architecture","file_type","block_count","embedding_length")): print(k,"=",t)
h=collections.Counter(); tot=0
for _ in range(nt):
    r.st(); nd=r.u32(); dims=[r.u64() for _ in range(nd)]; tt=r.u32(); r.u64()
    n=1
    for d in dims: n*=d
    nb=n//B.get(tt,1)*S.get(tt,4); h[T.get(tt,tt)]+=1; tot+=nb
print(h, f"{tot/1e6:.1f} MB")
```

## 3. LoRA-механика

**Файл:** `adapter.type=lora`, `adapter.lora_alpha=1.0`, `general.architecture=qwen35`, 128 тензоров, все **F32**, 2.37 MB. 64 сайта: 32×`ffn_down`, 24×`ssm_out`, 8×`attn_output` (слои 3/7/11/15/19/23/27/31). Имена: `blk.N.<site>.weight.lora_a [in,1]` и `.lora_b [1,out]`. Метаданные `prism.abliterate.*` (base PTQ1_0 sha, direction row 31, sites 64, target_embedding=no).

**API в 4838ffb** (`include/llama.h`): `llama_adapter_lora_init(model, path)`, `llama_adapter_lora_init_from_file_ptr(model, FILE*)` (удобно для SAF/content://), `llama_adapter_lora_free`, `llama_set_adapters_lora(ctx, adapters, n, scales)`, плюс `llama_adapter_meta_*` для чтения метаданных. CLI: `--lora` (список через запятую) и `--lora-scaled FNAME:SCALE,...`; сервер умеет `--lora-init-without-apply` и per-request `"lora":[{"id":0,"scale":1}]`.

**Как применяется:** `build_lora_mm()` в `llama-graph.cpp` считает базовый матмул (для hadamard-весов — по повёрнутому входу) и **отдельно** `B·A·x` в исходном базисе; веса не трогаются и не мержатся. Поэтому:
- работает поверх любых квантованных весов, включая Q2_0/TQ (проверено: адаптер поверх Q2_0 меняет поведение);
- `token_embd` запрещён явно (`Llama tensor … targets a prism.hadamard latent lookup table, which is not supported`), потому что таблица хранит повёрнутые строки;
- `scale = adapter_scale * alpha / rank` → здесь `1.0 * 1.0 / 1 = 1.0`, слайдер пользователя — прямой множитель;
- ограничений на тип тензоров адаптера в коде нет (shape-check + `ggml_dup_tensor`), но практически F32/F16; F32 — норма.

**Память/скорость:** адаптер целиком в RAM: 2.37 MB (в логах `LoRA buffer size ≈ 2.3 MiB`), поверх mmap-весов. Decode: 128 дополнительных rank-1 матмулов на токен — на десктопе 12.3 → 12.8–13.1 tok/s (шум, ≤5 %). Prefill — незаметно. Несколько адаптеров — да, `llama_set_adapters_lora` принимает массив; scale 0 = адаптер не добавляется в граф. Включение/выключение на лету — да, `set_adapters_lora` ставит `sched_need_reserve`, следующий `llama_process` строит граф заново; KV-кэш и контекст не сбрасываются.

**Safetensors/PEFT:** в этом репо не нужен. Если появится: `convert_lora_to_gguf.py --base <base.gguf> --outtype f32 adapter_dir/`, подводные камни — поддержка `qwen35` в скрипте конвертера, имена `model.layers.N.self_attn.o_proj` → `blk.N.attn_output`, `mlp.down_proj` → `ffn_down`, `linear_attn.out_proj` → `ssm_out`; shape A должна быть `[in, rank]`, B `[rank, out]` (иначе «lora_a tensor is not transposed»); `alpha/rank`; и **обязательно** пересчёт A в «unfolded input basis» (H·D), иначе на hadamard-паках дельта будет в неправильном базисе. Для готового GGUF всё уже сделано AtomicChat.

## 4. UX/интеграция в приложение

**Текущее состояние:** `snapcore.cpp` создаёт `engine{model, ctx, threadpool, tmpl, sampler…}`, `nativeInfo` возвращает `desc/size_mb/params_b/layers/ctx_train/ctx`. LoRA нет вообще. `ModelStore.scan()` берёт **все** `.gguf` — LoRA-файл в `models/ternary/` показывается как отдельная «модель», это надо чинить.

**Модель-пакет.** Каталог `models/<pack>/`:
```
base.gguf
adapters/uncensored-lora.gguf
pack.json  {base, adapters:[{path, name, license, scale, enabled, notes}]}
```
`ModelStore` читает `general.type`; `adapter` → в список адаптеров, не в модели. Для импорта через SAF — `llama_adapter_lora_init_from_file_ptr` (fdopen от `ParcelFileDescriptor`), чтобы не копировать 2 MB.

**JNI (минимальный вариант):**
```
nativeLoraLoad(handle, path) -> JSON {"ok":true,"sites":64,"alpha":1.0,"tensors":128,"mb":2.3}
nativeLoraSet(handle, enabled:Boolean, scale:Float) -> Int   // llama_set_adapters_lora
```
В `engine`: `llama_adapter_lora *lora; float lora_scale=1.f; std::string lora_path; bool lora_on`. `nativeFree`: сначала `llama_adapter_lora_free`, потом `llama_model_free` (иначе адаптер освободится вместе с моделью, но лучше явно). В `nativeInfo` добавить:
```json
"arch":"qwen35","quant":"Q2_0","hadamard":2,
"lora":{"path":"…","scale":1.0,"active":true,"sites":64,"mb":2.3}
```
Плюс `nativeLoraLoad` должен возвращать `prism.abliterate.base_gguf` и `base_sha256` — по ним показывать «адаптер для этой базы» и предупреждать при расхождении.

**Полный вариант:** массив адаптеров (`std::vector<llama_adapter_lora*>` + scales), per-chat override scale, индикатор «+2.3 MB RAM», кнопка «выгрузить», хранение `Map<modelPath, {adapterPath, scale, enabled}>` в `Prefs`/`pack.json`, авто-применение при `load()`. Per-request scale в одном процессе не нужен (n_seq_max=1), достаточно слайдера.

**Показ пользователю:** бейдж **«Uncensored (abliterated)»** + строка «Убирает обученный отказ; research-grade, MMLU 68.7→69.0 (n=300), не тестировался на изображениях. Apache-2.0. Создано AtomicChat на базе Prism ML Bonsai». Первое включение — диалог с подтверждением (18+/политика), в настройках — переключатель «Показывать uncensored-адаптеры». В `THIRD_PARTY_NOTICES.md` добавить Prism ML, Alibaba (Qwen3.5-4B), AtomicChat; в UI — ссылку на `LICENSE`/`NOTICE.txt`.

**Merge:** технически `llama-export-lora` есть, но для ternary-пака бессмысленен: rank-1 дельта при округлении до `{-1,0,+1}` почти вся исчезнет (об этом прямо пишет карточка адаптера). Только runtime.

## 5. Оценки на SDM845 (6 GB)

Измерено на десктопе (4 потока, ctx 4096, Q2_0, q8_0 KV, FA on):
`model mapped 1145 MiB`, `KV 68 MiB`, `RS 50 MiB` (R 2.25 + S 48), `compute 22 MiB`, output 1 MiB → **~1.29 GiB**, ~13 tok/s decode, ~20 tok/s prefill.

На MI 8 (CPU-only, 4×A75/A55, LPDDR4X) **[ГИПОТЕЗА]**: bandwidth ~8–10 GB/s и отсутствие dotprod → **decode 3–5 tok/s** (центральная 3.5–4), **prefill 5–8 tok/s**, TTFT на 100 токенов ~15–20 с. LoRA: +2.4 MB, −0…5 % decode. Память: 1.3–1.5 GiB RSS (mmap даёт file-backed, PSS ниже), при ctx 2048 — минус ~35 MiB KV. Это существенно легче Clover (0.6 tok/s, 12 GB файл), но в 6–8 раз медленнее Qwen2.5-0.5B.

## 6. План bring-up

1. **Скачать и сверить** (на ПК; в репо уже лежит, но перепроверить):
```powershell
curl.exe -L -o Ternary-Bonsai-2-4B-Q2_0.gguf https://huggingface.co/prism-ml/Ternary-Bonsai-2-4B-gguf/resolve/main/Ternary-Bonsai-2-4B-Q2_0.gguf
curl.exe -L -o Ternary-Bonsai-2-4B-Uncensored-LoRA.gguf https://huggingface.co/AtomicChat/Ternary-Bonsai-2-4B-Uncensored-LoRA-GGUF/resolve/main/Ternary-Bonsai-2-4B-Uncensored-LoRA.gguf
Get-FileHash Ternary-Bonsai-2-4B-Q2_0.gguf -Algorithm SHA256   # 64A5183B…9433D4
Get-FileHash Ternary-Bonsai-2-4B-Uncensored-LoRA.gguf -Algorithm SHA256  # 605E9DFB…38A9
```
2. **Десктопная проверка** (уже выполнена, повторяемая):
```powershell
llama-cli -m …Q2_0.gguf -p "Hello" -n 16 -t 4 -c 512 --temp 0
llama-cli -m …Q2_0.gguf --lora-scaled "lora.gguf:1" -p "…" -n 16 -t 4 -c 512 --temp 0
```
Внимание: `--lora-scaled` режет строку по `:`, поэтому **нельзя** `C:\...`; запускать из каталога адаптера с относительным именем.
3. **На устройстве** (не трогая APK): `adb push` модель и адаптер в `/data/local/tmp/sllm-bench/`, затем
```powershell
adb shell "cd /data/local/tmp/sllm-bench && LD_LIBRARY_PATH=. ./llama-completion -m Ternary-Bonsai-2-4B-Q2_0.gguf --lora-scaled lora.gguf:1 -p 'Hello' -n 16 -t 4 -c 512 --temp 0"
```
4. **Интеграция в APK**: фильтр `general.type=adapter` → `nativeLoraLoad/nativeLoraSet` → `nativeInfo.lora` → `Prefs` → UI toggle/scale → лицензии. Порядок именно такой: сначала CLI, потом JNI, потом UI.
5. **Критерии успеха:** модель грузится <5 с (mmap), вывод связный, `nativeInfo.quant=Q2_0`, адаптер меняет ответ на провокационный промпт, scale 0 возвращает базовое поведение, decode ≥3 tok/s, RSS <1.6 GiB, 20 минут генерации без OOM/ANR.
6. **При ошибке:** `invalid ggml type 143/142` → взят PTQ1_0/PQ2_0, скачать Q2_0; `unsupported prism.hadamard.version` → бинарник старее b11556/Prism v2; `model arch and LoRA arch mismatch` → не тот адаптер; `tensor … has incorrect shape` → другая база; OOM → ctx 2048, KV q8_0/q4_0, `mmap=true`, `nativeTrim`.

## 7. Риски

- **Лицензии:** база и адаптер Apache-2.0, но `NOTICE.txt` требует атрибуции Prism ML; Qwen3.5-4B — Alibaba. Адаптер — производная от базы; при распространении APK/моделей нужны NOTICE и текст лицензии. Логотипы/кнопки AtomicChat в README — не тащить в UI.
- **Uncensored:** снимает обученный отказ; модель будет отвечать на то, на что базовая не отвечает. Нужны age-gate/подтверждение, запрет в детских профилях, дисклеймер «не гарантирует безопасность/законность», возможность выключить. Политики Google Play к таким «research»-адаптерам относятся настороженно.
- **Качество:** 4B ternary Q2_0 — это не Qwen2.5-0.5B-класс по скорости и не 7B по уму; MMLU 68.7 (n=300, ±2.7) — ориентир, не гарантия. Русский/код не измерялись.
- **Hadamard:** поддерживается только v1/v2 и только для 6 архитектур; v3 или смена `transform` → отказ загрузки. LoRA в «primal basis» — при обновлении паков базис может разойтись, о чём предупреждает карточка (leak ~1e-4).
- **Типы:** PTQ1_0/PQ2_0 в этом форке нечитаемы; не пытаться «скормить» их в APK — пользователь получит непонятную ошибку. Нужна предварительная проверка типов в `GgufMeta` (сейчас читает только метаданные), чтобы показывать «формат не поддерживается этим билдом».
- **LMK/термика:** 1.3 GiB + Android — в пределах 6 GB, но длинный prefill на 4 ядрах греет SoC; при троттлинге decode может упасть до 2 tok/s. `onTrimMemory` уже чистит KV — адаптер при этом лучше не выгружать (2 MB).
- **Скорость:** главный риск — не память, а Q2_0 NEON-ядро и GDN-слои; если реальные 2 tok/s, модель останется «демо», а не daily driver.

**Итог:** Q2_0 + этот LoRA — единственная жизнеспособная связка в 4838ffb, она уже проверена на десктопе и меняет поведение; для телефона остаётся измерить скорость и добавить два JNI-вызова.
