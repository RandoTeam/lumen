# Отчёт 03. Экспертный кэш и I/O (полная версия)

Источник: параллельное исследование проекта SnapdragonLLM, 2026-11-07.
Устройство: MI 8, SD845, 6 GB, UFS 2.1, Android 15.

## TL;DR

- **Формат `expert-profile.bin` разобран полностью**, но это **не профиль Clover**. Заголовок `STRP`, v1: 48 строк × 512 столбцов = **24 576 пар**, а у Clover 40 слоёв × 256 экспертов = **10 240**. В глобальном top-5 есть строки 46, 47, 44, 45 — они не могут быть padding’ом 40-слойной модели. Это профиль модели 48×512 (по всем признакам Qwen3.8-Flash-Next из того же референса). Для Clover его нельзя применять без перегенерации.
- В llama.cpp 4838ffb на CPU-only работает **только mmap + page cache**. `llama-moe-cache` — это device-кэш: конструктор требует GPU/IGPU, иначе бросает `MoE cache requires a GPU backend`. `--moe-cache-mib` на CPU — no-op; включение скорее уронит загрузку. Слот-механика (`get_rows(slot_map)` + `ggml_mul_mat_id(..., slots)`) уже есть и CPU-ядро `MUL_MAT_ID` поддерживает `-1` (пропуск), так что host-пул реализуем поверх неё.
- Измерено: UFS 2.1 даёт **539 MB/s** последовательно, **490 MB/s** на случайных 1 MiB, **~5 000 IOPS** на 4 KiB; token-подобная нагрузка 360 MB — **265 MB/s в 1 поток, 531 MB/s в 2, 704 MB/s через O_DIRECT в 2 потока**. mmap по резидентным страницам — 2.4–2.5 GB/s, тёплый последовательный скан — 8.6 GB/s.
- Page cache сейчас держит **19.6 % файла (2.35 GiB)**. Под нагрузкой (peak RSS 4.26 GB) на экспертов остаётся примерно **1.7–1.9 GiB** суммарно под пул + page cache.
- Стартовый дизайн: **пул 24 слота/слой ≈ 966 MiB**, O_DIRECT-pread в слоты, 2 I/O-потока, admission 2-hit, prefill bypass. Ожидаемо **0.95–1.1 tok/s** (оптимистично 1.2–1.35, пессимистично 0.70–0.80). Потолок ~1.1–1.3 tok/s: dense-часть уже читает ~1.4 GB/токен, и один экспертный кэш её не уберёт.

## 1. Разбор `expert-profile.bin`

Файл 196 632 байта. Структура:

```
0x00  char[4]  "STRP"
0x04  u32      version = 1
0x08  u32      n_rows  = 48
0x0C  u32      n_cols  = 512
0x10  u32      n_cells = 24576
0x14  u32      n_cells = 24576   (дубль/длина второго массива)
0x18  ...      payload
```

Payload ровно 196 608 байт = два массива по 24 576 × 4 байта:

1. **`order[rank] = (row:u16, col:u16)`** — глобальный порядок «горячности»: `rank=0` самый горячий. Это перестановка всех 24 576 клеток сетки 48×512 (проверено: все пары уникальны, покрывают сетку ровно один раз).
2. **`rank[row*512+col] = u32`** — обратная перестановка: позиция клетки в `order`. Проверено: `rank[flat(order[i])] == i` для всех i.

Никаких частот, счётчиков, top-k или имён тензоров в файле нет — только тотальный порядок.

Парсер:

```python
import struct

def parse_strp(path):
    b = open(path, "rb").read()
    magic, ver, nrow, ncol, n, n2 = struct.unpack_from("<4sIIIII", b, 0)
    assert magic == b"STRP" and ver == 1 and n == nrow*ncol == n2
    off = 24
    order = [struct.unpack_from("<HH", b, off + 4*i) for i in range(n)]
    rank  = [struct.unpack_from("<I",  b, off + 4*n + 4*i)[0] for i in range(n)]
    assert len(set(order)) == n
    assert all(rank[r*ncol + c] == i for i, (r, c) in enumerate(order))
    return nrow, ncol, order, rank

nrow, ncol, order, rank = parse_strp("models/clover/expert-profile.bin")
```

**Ранжирование.** Глобальный top-5: `(28,288)`, `(46,185)`, `(47,242)`, `(33,117)`, `(44,76)`. Внутри строк порядок тоже задан, например:

| row | top-8 экспертов |
|---|---|
| 0 | 93, 349, 251, 278, 287, 384, 20, 79 |
| 28 | 288, 21, 334, 228, 372, 80, 6, 473 |
| 46 | 185, 232, 346, 374, 85, 73, 128, 390 |
| 47 | 242, 398, 112, 333, 323, 29, 89, 317 |

Строки 40–47 содержат глобальные rank 1, 2, 4, 18 и т.д. Это окончательно доказывает: **48×512 — не Clover**. В логе 4gbVramStrata для Clover прямо сказано «244 of 10 240 routed experts», и `expert-profile.txt` там был 40×256. Наш `.bin` — профиль другой модели (24 576 экспертов, 48×512), вероятнее всего Qwen3.8-Flash-Next. **Вывод: файл можно использовать как пример формата и как источник идей о skew, но не как карту горячих экспертов Clover.** Для Clover нужен профиль 40×256, снятый с роутера этой модели, либо онлайн-счётчики.

## 2. Что делает llama.cpp 4838ffb на CPU-only

- **Загрузка.** `snapcore.cpp` ставит `load_mode=MMAP`, `lazy_mode=AUTO`. `llama_model.cpp` вызывает `ml.init_mappings(true, ...)`, а `llama-model-loader.cpp` передаёт `prefetch_size = -1`. В `llama-mmap.cpp` это превращается в `MAP_POPULATE` (если `lazy_ranges` пуст) и `POSIX_MADV_WILLNEED` на **весь файл**. Плюс `POSIX_FADV_SEQUENTIAL` на fd. Для 12.5 GB на 6 GB это выглядит как преждевременное вытеснение page cache и лишний I/O на старте. В логах устройства я это не поймал (лог прокручен), поэтому помечаю как **анализ по коду, требует подтверждения экспериментом E0b**.
- **Веса.** Экспертные тензоры 3 на слой: `ffn_gate_exps`, `ffn_up_exps`, `ffn_down_exps`, формы `[2048,512,256]` / `[512,2048,256]`. Типы: IQ3_XXS, IQ2_S, Q3_K. Суммарно **10 304 MiB** экспертов; не-экспертная часть **1 672 MiB** (output 398, token_embd 273, attn/ssm 842, shared FFN 148, norms 11). Размер эксперта по слоям: 1.195 MiB (0,1,38,39), 1.023 MiB (2–15), 0.961 MiB (16–37). Среднее 1.006 MiB.
- **MoE-путь.** `build_moe_ffn` → `build_moe_cache_slots` → `ggml_get_rows(slot_map, ids)` → `ggml_mul_mat_id(ctx, bank, cur, slots)`. Если `moe_cache == nullptr`, используется обычный `ggml_mul_mat_id(w, cur, ids)` с `w->data`, указывающим в mmap. Тогда каждый промах — page fault внутри матмула.
- **`llama-moe-cache`.** Это **device-кэш**: в конструкторе отбираются только `GGML_BACKEND_DEVICE_TYPE_GPU/IGPU`; если список пуст — `throw std::runtime_error("MoE cache requires a GPU backend")`. `--moe-cache-mib` > 0 на CPU приведёт к исключению при создании контекста. В `docs/14-BACKENDS.md` это уже зафиксировано: «`moe_cache_mb` на CPU — no-op».
- **Что переиспользуемо.** `moe_cache_slru` (2Q/SLRU, `seen`-защита от вытеснения экспертов текущего запроса), `plan()`, `build_moe_cache_slots`, слот-семантика `MUL_MAT_ID`. CPU-ядро `ggml_compute_forward_mul_mat_id` явно поддерживает `id == -1` (обнуляет строку), значит slot map обязан быть полным до вычисления.
- **Ограничение.** `copy_experts()` для больших батчей пишет в `dst` (host-тензор). На mmap это read-only отображение — писать нельзя. Для host-пула этот путь надо отключить (`return 0`) и оставить bypass для `n_tokens > 32`.

## 3. Измерения на устройстве

Условия: MI 8, Android 15, app idle (RSS 619 MB), батарея 79 %, 23 °C, PSI memory `avg10=0.05 %`. Все чтения — только чтение, запись на диск не выполнялась. Модель не изменялась.

| Тест | Условие | Результат |
|---|---|---|
| seq read 512 MiB | `O_DIRECT`, 1 поток | 950.3 ms = **538.8 MB/s** |
| seq read 256 MiB | buffered, offset ~5987 MiB, холодный | 514.7 ms = **497.4 MB/s** |
| тот же диапазон сразу | buffered, горячий | 134.3 ms = **1906 MB/s** |
| random 1 MiB ×24 | `O_DIRECT` | p50 **1.917 ms** (p90 2.164), ~**490 MB/s** |
| random 4 KiB ×48 | `O_DIRECT` | p50 **0.201 ms** (~4 975 IOPS, 19.4 MB/s) |
| random 1 MiB ×24 | buffered + `fadvise(DONTNEED)` | p50 2.849 ms (в другом прогоне 1.08 ms на 1.1 MiB) |
| random 1 MiB, горячий | buffered | p50 0.504 ms (~2.0 GB/s) |
| random 4 KiB, горячий | buffered | p50 0.001 ms (page cache) |
| mmap 1.1 MiB, резидентные страницы | `madvise(DONTNEED)` + touch | p50 3.935 ms; в цикле 320 чтений — **0.47 ms/expert** (2.4 GB/s) |
| mmap seq 256 MiB, холодный | touch после `madvise` | 483.6 ms = **529 MB/s** |
| mmap seq 256 MiB, горячий | touch | 29.7 ms = **8626 MB/s** |
| `MADV_WILLNEED` 64 MiB | issue 0.3 ms, затем touch | 103.6 ms = **616 MB/s** |
| pread 1.1 MiB, холодный | buffered + `fadvise` | p50 **1.08 ms** |
| pread 1.1 MiB, горячий | buffered | p50 0.693 ms |
| token-нагрузка 360 MB | pread, **1** поток | 1356.7 ms = **265 MB/s** |
| token-нагрузка 360 MB | pread, **2** потока | 678.5 ms = **531 MB/s** |
| token-нагрузка 360 MB | `O_DIRECT`, **2** потока | 511.4 ms = **704 MB/s** |
| page cache residency | `mincore` по всему файлу | **601 658 / 3 065 764 страниц = 19.6 % = 2.35 GiB** |

Память/состояние: `MemTotal 5 766 584 kB` (5.50 GiB), `MemAvailable 3.65 GB`, `Cached 3.71 GB`, `SwapTotal 2.29 GB / used 0.69 GB`, zram used 722 MB, `AnonPages 622 MB`, `Mapped 1.01 GB`. `dumpsys meminfo`: app 619 MB RSS (idle), system 403 MB, wellbeing 303 MB, systemui 184 MB. Термозоны 21.6–25.6 °C (max 25.6), battery 23 °C, 4.02 V.

**Что это значит.** Резидентность 2.35 GiB при `Cached` 3.71 GB — это и есть «page cache, который ОС отдаёт под mmap» в idle. При inference peak RSS 4.26 GB, значит под file cache остаётся заметно меньше. O_DIRECT в 2 потока даёт 704 MB/s и **не загрязняет page cache** — это лучший путь для заполнения пула. `MADV_WILLNEED` дёшев в issue (0.3 ms) и асинхронен, но данные приходят на ~616 MB/s; для 1 MiB это всё равно ~1.6–2 ms, поэтому «prefetch по требованию» не бесплатен.

## 4. Бюджет на 6 GB

| Статья | MiB |
|---|---:|
| MemTotal | 5 499 |
| Ядро/Android/модем/Adreno reserve (оценка) | 1 300–1 450 |
| Dense (mmap, читается каждый токен): output 398 + embd 273 + attn/ssm 842 + shared 148 + norms 11 | **1 672** |
| Compute buffer (факт) | 127.5 |
| Recurrent state (факт) | 62.8 |
| KV q8_0, ctx 4096 (оценка: 10 full-attn слоёв × 2 KV-head × 256 × 2 × 4096) | ~45 |
| Код/JNI/стеки/аллокатор | 120–180 |
| Safety margin | 250–350 |
| **Остаётся на экспертов (пул + page cache)** | **1 700–1 900** |

Проверка: `1450+1672+128+63+45+150+300 = 3808`; `5499−3808 = 1691 MiB`. Это согласуется с peak RSS 4.26 GB, где файловые страницы экспертов занимали ~2 GB.

Варианты пула (точные размеры по слоям):

| Слотов/слой | Размер пула | Доля от 10 240 |
|---:|---:|---:|
| 20 | **805 MiB** | 7.8 % |
| 24 | **966 MiB** | 9.4 % |
| 28 | **1 127 MiB** | 10.9 % |

Расчёт 24: 4 слоя × 24 × 1.195 = 114.7; 14 × 24 × 1.023 = 343.7; 22 × 24 × 0.961 = 507.4; итого **965.8 MiB** + 40 KB slot map.

Рекомендация: **24 слота/слой (966 MiB)** + целевой page cache 600–800 MiB + margin 250–300 MiB. Fallback — 20 слотов (805 MiB), если PSI/LMK начнёт расти.

## 5. Три дизайна

### A. Только page cache + `madvise(WILLNEED)`

Патчи:
- `src/llama-model-loader.cpp::init_mappings`: не передавать `prefetch=true` для 12.5 GB файла; отключить `MAP_POPULATE`/`WILLNEED` всего файла. Для экспертных диапазонов — `POSIX_FADV_RANDOM` вместо `SEQUENTIAL`.
- `src/llama-graph.cpp::build_moe_ffn`: после `selected_experts` (на CPU ids уже в host-памяти, синхронизация не нужна) вызвать `moe_io->willneed(il, ids)`.
- `llama-model-loader`: сохранить `(fd, tensor_off, nb[2])` для экспертных тензоров.

```cpp
// после top-k, ids — host int32
for (int e : ids)
    madvise(base + off_gate + e*nb2, nb2, MADV_WILLNEED),
    madvise(base + off_up   + e*nb2, nb2, MADV_WILLNEED),
    madvise(base + off_down + e*nb2, nb2, MADV_WILLNEED);
```

Плюсы: 1–2 дня, нет нового расхода RAM, нет OOM. Минусы: page cache не подчиняется нам, при 6 GB его вытесняет сам inference; fault остаётся дорогим; спекулятивный prefetch может жечь UFS. Ожидаемый hit — как сейчас, 20–30 %.

### B. Явный RAM-пул поверх mmap

Расширить `llama_moe_cache` режимом `HOST`:
- разрешить CPU-backend (сейчас `devices.empty()` → throw);
- банки — host-буферы, по `n_slots` на слой, slot map `I32[256]`;
- в `copy()` вместо `ggml_backend_tensor_set_async` делать `pread` из fd в банк;
- `copy_experts()` для host-режима вернуть 0 (нельзя писать в mmap);
- `get_slot_map()` возвращает map только при `n_tokens <= 32`, иначе nullptr → обычный mmap-путь.

Псевдокод:

```cpp
// loader: io[t] = {fd, off, stride=nb[2], n=ne[2]}
// llm_graph: slots = get_rows(slot_map, ids)
// copy():
memcpy(ids, sel->data, nbytes);           // CPU: без sync
plan(il, ids, n, fills, n_hit);          // 2Q/SLRU, per-layer
for (fill f : fills)
    pread_odirect(io[t], off + f.expert*stride,
                  stride, bank[t] + f.slot*stride);
wait_all();                               // 1–2 I/O потока
ggml_backend_tensor_set(slot_map_tensor, ...);
```

`pread_odirect` с bounce на 4 KiB, потому что базы тензоров выровнены на 128 B, а не на 4096:

```cpp
aoff = off & ~4095; delta = off - aoff;
pread(fd, bounce + delta, round_up(len + delta, 4096), aoff);
memcpy(dst, bounce + delta, len);
```

Плюсы: детерминированная память, per-layer справедливость, O_DIRECT 704 MB/s, нет fault’ов, hit управляем. Минусы: копирование на промахе, больше кода, нужен аккуратный pipeline.

### C. Гибрид: hot set в пуле, cold через mmap, prefetch L+1

- 24 слота/слой — только admitted-hot.
- Промах, не прошедший admission, читается в transient-кольцо 2 слота/слой или напрямую через mmap (не занимает пул).
- Пока считается слой L, по онлайн-счётчикам (или валидному профилю) выпускается `WILLNEED` на top-2 слоя L+1; текущие 8 miss’ов L читаются 2 потоками и pipeline’ятся.
- Admission: 2 попадания в окне 256 токенов; aging — halving каждые 256 токенов; protected-сегмент 50 %.

```cpp
for (il = 0; il < 40; ++il) {
    ids = router(il);                       // host, без sync
    miss = ids where slot_map[il][e] < 0;
    io_start(il, miss);                     // 2 threads, O_DIRECT
    if (idle) willneed_next(il+1, top2_online(il+1));
    io_wait(il);
    compute_moe(il, slots);
    update_frequency(il, ids);
}
```

Плюсы: лучший ожидаемый hit и overlap. Минусы: самый сложный, спекуляция может вредить. Это фаза 2 после B.

**Как читать top-k без полной синхронизации.** На CPU роутер и top-k уже считаются на CPU; `sel->data` — обычная host-память. Трюк llama.cpp: `slot_map` — host weight, поэтому планировщик делает split перед `GET_ROWS`; `copy()` вызывается на границе split’а, когда `sel` уже посчитан. Дополнительный `ggml_backend_synchronize` на CPU — это фактически memcpy, не простой. Для prefetch L+1 нужен отдельный хук в `build_moe_ffn`, но ids L+1 там ещё не существует; поэтому L+1 — только спекуляция по счётчикам/профилю, а не «настоящий» top-k.

## 6. Параметры старта

| Параметр | Значение | Обоснование |
|---|---|---|
| Слотов на слой | **24** (fallback 20) | 966 MiB; 9.4 % всех экспертов; per-layer квота лучше глобального LRU |
| Размер слота | точный `nb[2]` слоя (0.961–1.195 MiB) | Не усреднять: слои 0,1,38,39 на 24 % больше |
| Общий пул | **966 MiB** | Влезает в slack 1.7–1.9 GiB вместе с page cache |
| Admission | **2-hit / 256 токенов** | Не засорять пул одноразовыми экспертами |
| Eviction | per-layer 2Q/SLRU, protected 50 % | Уже реализовано в `moe_cache_slru` |
| Prefetch | текущий слой: все 8 miss’ов, pipeline 1 эксперт; L+1: top-2 `WILLNEED` только при idle | 2 эксперта × 40 × 1 MiB = 80 MiB спекуляции; включать при precision > 30 % |
| I/O | **O_DIRECT + 4 KiB bounce**, 2 потока, queue depth 4 | 704 MB/s против 531 MB/s buffered; не дублирует page cache |
| Потоки | compute 4 (gold), I/O 2 (silver, nice +10) | 845: 4×A75 — оптимум; A55 не должны мешать |
| Prefill | bypass при `n_tokens > 32` | Большие батчи выбирают почти все эксперты, кэш только навредит |
| madvise vs pread | pread — для пула; `WILLNEED` — только спекуляция | pread 1.08 ms/1.1 MiB холодный; WILLNEED асинхронен, но 616 MB/s |
| Safety | при `MemAvailable < 600 MB` — перестать admission; `< 300 MB` — выключить пул | Не провоцировать LMK/zram thrash |

## 7. Прогноз (модель, не измерение)

Текущий decode 0.60–0.67 tok/s = 1.49–1.67 s/токен. Оценка трафика: эксперты ~320–350 MB + dense ~1.4 GB (output 398 + attn/ssm 842 + shared 148 + embd/norms) ≈ 1.75 GB/токен. При 0.6 tok/s это ~1.05 GB/s эффективной памяти — смесь page cache и диска.

| Сценарий | Пул hit | Суммарный hit | Miss, MB/токен | I/O, ms | Decode, tok/s |
|---|---:|---:|---:|---:|---:|
| Оптимистичный | 60 % | 65 % | ~122 | ~175 | **1.20–1.35** |
| Реалистичный | 42 % | 50 % | ~175 | ~250 | **0.95–1.10** |
| Пессимистичный | 20 % | 25 % | ~262 | ~375 | **0.70–0.80** |

Почему не больше: даже при 100 % hit остаётся dense ~1.4 GB/токен. При 2.4 GB/s (mmap по резидентным страницам) это ~580 ms, плюс compute ~300–500 ms → потолок **~1.1–1.3 tok/s**. Чтобы уйти выше, нужно уменьшать output head (398 MiB Q6_K), attn/ssm или использовать спекуляцию/GPU — это уже не задача экспертного кэша.

## 8. Матрица экспериментов

| ID | Что меняем | Метрики | Критерий успеха |
|---|---|---|---|
| E0 | baseline: mmap, 4 потока, ctx 4096, KV q8_0 | decode, TTFT, PSS, hit (по `mincore`/faults) | 5 прогонов, медиана |
| E0b | отключить whole-file `MAP_POPULATE`/`WILLNEED`, `FADV_RANDOM` на экспертов | время загрузки, residency, decode | загрузка быстрее, hit не хуже |
| E1 | A: `WILLNEED` top-4 текущего слоя | minor faults, decode, SSD MB | decode +10 % без роста PSS |
| E2 | B: пул 16/20/24, admit-all vs 2-hit, pread buffered vs O_DIRECT | hit rate, SSD MB/токен, decode | hit ≥ 30 %, decode ≥ 0.85, PSS ≤ 4.3 GB |
| E3 | C: B + L+1 top-2 `WILLNEED` + 2 I/O потока | hit, I/O wait, decode | hit ≥ 40 %, decode ≥ 1.0 |
| E4 | 1/2/3 I/O потока; bounce 4 KiB vs 512 B; O_DIRECT vs buffered | MB/s, CPU steal, decode | 2 потока, O_DIRECT |
| E5 | admission threshold 1/2/3, aging 128/256/512 | hit, churn, decode | лучший hit без роста SSD MB |

Логировать на токен/слой: `hits`, `misses`, `read_mb`, `willneed_mb`, `io_wait_ms`, `faults`; на прогон: `expert_hit_rate`, `ssd_read_mb` (из `/proc/self/io read_bytes`), `decode_tok_s`, `ttft_ms`, `peak_pss`, `zram_delta`, `thermal`, `lmk_kills`. Quality gate — те же 22 промпта, иначе кэш может «улучшить» скорость, поменяв ответы.

**Если hit rate < 30 %:**
1. Проверить, что профиль вообще валиден для 40×256 (сейчас — нет).
2. Переключить admission на admit-all LRU и уменьшить пул до 16 слотов, вернув память page cache.
3. Откатиться на дизайн A и проверить, не даёт ли page cache больше.
4. Снять реальный профиль Clover: 2–4 тыс. токенов, логировать ids роутера, построить 40×256 порядок, залить top-16/layer на старте.
5. Если и это < 30 % — routing плоский, экспертный кэш не рычаг. Тогда оптимизировать dense: output head (Q4/Q5), KV q4_0, спекуляция, chunked prefill; или признать, что 6 GB CPU-only упирается в dense-трафик.

## 9. Риски

- **Профиль не тот.** 48×512 ≠ 40×256; применение «как есть» даст мусорный hot set. Это главный риск.
- **O_DIRECT alignment.** Базы тензоров выровнены на 128 B (у слоёв 1+ смещение 1152/2304 mod 4096), поэтому прямой pread упадёт с `EINVAL`; нужен 4 KiB bounce.
- **Дублирование памяти.** Buffered pread в пул дополнительно кладёт данные в page cache; без O_DIRECT/`fadvise(DONTNEED)` пул и кэш будут конкурировать.
- **Запись в mmap.** `copy_experts()` для больших батчей пишет в `dst`; на host-mmap это segfault. В host-режиме отключить.
- **LMK/zram.** Пул анонимный; при нехватке памяти ядро начнёт свапить 1 GB в zram (2.29 GB, уже 0.69 GB used). Следить за `MemAvailable`, PSI, `SwapFree`; при `< 600 MB` — сжимать пул.
- **Термика.** Сейчас 22–26 °C в idle; устойчивый decode на 4×A75 нагреет сильнее. 2 I/O потока на silver, не на gold.
- **Prefetch вредит.** Спекуляция L+1 при точности < 30 % читает лишние ~80 MB/токен и отбирает UFS у demand-чтений.
- **Whole-file prefetch.** Если `MAP_POPULATE`/`WILLNEED` на 12.5 GB действительно срабатывает, старт и page cache уже испорчены; E0b — самый дешёвый первый шаг.
- **Prefill regression.** Пул не должен трогать батчи > 32 токенов; иначе TTFT вырастет.
