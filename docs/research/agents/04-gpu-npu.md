# Отчёт 04. Vulkan, OpenCL, NPU (полная версия)

Источник: параллельное исследование проекта SnapdragonLLM, 2026-11-07.
Устройство: MI 8, SD845, Adreno 630, Android 15.

## TL;DR

- **845 / Adreno 630: Vulkan буста не даст.** Субагент собрал Vulkan-backend из `4838ffb` и проверил на устройстве. Backend видит GPU (`Adreno 630, fp16:0, int dot:0, matrix cores:none`), но на `-ngl 99` падает с `device Vulkan0 does not support 16-bit storage`. Патч «fp32-only» (аналог PR #27723) снимает проверку, но бинарник сегфолтится в `ggml_vk_create_buffer`: драйвер 845 не тянет даже аллокацию буферов. Это не «надо ещё немного потюнить», это закрытая дверь.
- **OpenCL на 845 уже измерен: 3.05 pp / 2.23 tg против 41.8 / 26.6 на CPU.** Причина не в том, что «GPU слабый»: 0.5B Q4_K на CPU читает ~10 GB/s из 29.9 GB/s LPDDR4X, то есть даже идеальный GPU ограничен примерно 3x. 12-14x провал — это generic-ядра, отсутствие Adreno-интринсиков, launch overhead и старый драйвер, а не пропускная способность.
- **Clover 35B-A3B на 845 вообще не про compute.** 11.69 GB модель, 6 GB RAM, decode 0.6 t/s при ~200 MB/s эффективного чтения. Это UFS/page cache, а не CPU/GPU/NPU. Никакой backend это не исправит.
- **8 Gen 3 / 8 Elite:** GPU и NPU имеют смысл, но **prefill**, а не обязательно decode. OpenCL на Adreno 750: Qwen2.5-1.5B Q4_0 — 252 pp / 24.1 tg; 3B — 136 pp / 21.4 tg. Vulkan на Adreno 750/830 имеет известные падения драйвера. NPU (Hexagon v75/v79) — самый перспективный путь, но это отдельный проект.
- **Главный вывод:** на 845 оставить CPU 4 потока; на новых Snapdragon сначала мерить CPU, потом OpenCL/NPU, Vulkan — только с SDK и патчами. Clover на 845 — storage-demo; на 8 Gen 3 16 GB — на грани по RAM; на 8 Elite 24 GB — уже осмысленно.

## 1. Физика: почему OpenCL оказался в 12-14 раз медленнее

**Факт.** Decode — memory-bound: за токен надо прочитать почти все веса. Qwen2.5-0.5B Q4_K_M весит 0.37 GB; 26.6 tok/s дают ~9.8 GB/s. LPDDR4X-3733 на SDM845 — 4x16 bit, ~29.9 GB/s теоретически. Значит потолок для любого backend на 845 — примерно 3x от CPU, реалистично 1.2-1.8x. Числа OpenCL (2.23 tok/s, ~450 ms/токен) в 30 раз выше памяти-пола (0.37 GB / 29.9 GB/s ≈ 13 ms). То есть 97% времени — не чтение весов.

**Факт.** Generic OpenCL-ядра не используют Adreno-специфику: `dot_acc_sat_4x8packed_ss_int` и `qcom_wave_pair_mode` драйвер 630 не компилирует. Q4_K деквантуется в f32 и считается обычными FMA, без packed dot.

**Факт.** На 845 нет dotprod/i8mm: наши CMake-проверки для target дали `HAVE_DOTPROD - Failed`, `HAVE_MATMUL_INT8 - Failed`. Нет и FP16 compute в Vulkan (`fp16:0`).

**Гипотеза (сильная).** Основной убийца decode — launch/sync overhead: сотни мелких dispatch на токен, старый Adreno-драйвер, отсутствие батчинга и subgroup-оптимизаций. Деквант Q4_K и отсутствие dotprod — второй фактор. Пропускная способность — третий: она ограничивает потолок, но не объясняет 12x.

**Что принципиально, а что чинится:**
- **Принципиально на 845:** UMA-память (GPU и CPU делят 29.9 GB/s), отсутствие dotprod/матричных блоков, EOL-драйвер, batch=1.
- **Чинится софтом:** деквант (Q4_0 вместо Q4_K, repack), tiling, command-buffer batching, специализированные GEMV-ядра. Но на 845 это всё равно упрётся в драйвер и память.
- **Prefill** — compute-bound и теоретически GPU-friendly, но generic OpenCL дал 3.05 tok/s (~1.4 GFLOPS, доли процента пика). Это провал реализации, а не железа. Vulkan #15800 показывает то же на Mali: generic Vulkan `pp512` 2.81 tok/s, специализированное ядро — 66 tok/s.

## 2. Факты по Vulkan на Adreno 630 / Android 15

**Что проверено на устройстве (факт):**
```
ggml_vulkan: 0 = Adreno (TM) 630 | uma: 1 | fp16: 0 | bf16: 0 | fp4: 0
              warp size: 64 | shared memory: 32768 | int dot: 0 | matrix cores: none
```
`--list-devices` работает. Но `llama-bench -ngl 99` даёт:
```
ggml_vulkan: device Vulkan0 does not support 16-bit storage.
llama_model_load: error loading model: Unsupported device
```
В коде `ggml_vk_get_device()` жёстко требует `vk11_features.storageBuffer16BitAccess`. В `4838ffb` это ~строка 4664; в upstream условие то же. Плюс `ggml_vk_instance_init()` требует Vulkan 1.2 (строка ~5225). Android-манифест устройства декларирует `android.hardware.vulkan.version-1_1` и `compute level 0`, но loader на LineageOS пропускает проверку 1.2; физическое устройство остаётся 1.1 без 16-bit storage.

**Патч fp32-only (PR #27723).** Субагент собрал копию в `build/llama-vulkan/src-exp`, снял требование 16-bit storage для `fp16=false`. Результат: `SIGSEGV` в `ggml_vk_create_buffer` (null driver entry point) при загрузке модели. То есть даже обход проверки не даёт рабочего пути.

**Что поддерживает Vulkan-backend (факт по коду):** Q4_K/Q5_K/Q6_K, IQ1-IQ4, TQ1/TQ2, MoE (`MUL_MAT_ID`, `topk_moe`), flash attention, `moe_cache_size` (в дереве есть `llama-moe-cache.cpp`). Но на 845 всё это не имеет значения без 16-bit storage/fp16.

**Известные проблемы на новых Adreno (факты/ссылки):**
- Adreno 750 (8 Gen 3): драйвер сегфолтится в `vkCreateComputePipelines` на matvec-пайплайнах; нужен opt-in guard — https://github.com/ggml-org/llama.cpp/pull/29165
- Adreno 750: flash attention `cols_per_thread=0` — https://github.com/ggml-org/llama.cpp/issues/30391
- Adreno 830 (8 Elite): vendor-драйвер делает SIGABRT на `-ngl>=1`, Turnip работает — https://github.com/ggml-org/llama.cpp/issues/29786
- Adreno 830: SPIR-V от NDK/shaderc одной версии не компилируется, от другой — работает — https://github.com/ggml-org/llama.cpp/issues/28635
- Adreno 830: неверные результаты quantized `MUL_MAT` — https://github.com/ggml-org/llama.cpp/issues/30654
- Adreno 840: crash на TQ1_0 27B — https://github.com/ggml-org/llama.cpp/issues/30649
- KGSL throttling между submissions бьёт по tg; патч даёт +50% на MoE — https://github.com/ggml-org/llama.cpp/issues/27191
- 8-bit pack/unpack мисскомпилируется на нескольких Adreno — https://github.com/ggml-org/llama.cpp/pull/30821
- fp32-only устройства (Haswell) — https://github.com/ggml-org/llama.cpp/pull/27723

**Вывод по 845:** Vulkan не «медленный», он **нерабочий** на этом драйвере. Даже если допатчить, потолок — та же LPDDR4X и старый GPU; ожидание ≤ CPU. Тратить время нельзя.

## 3. Toolchain: что есть и что реально собралось

**Факты окружения:**
- NDK 28.2: `glslc.exe` есть, но это shaderc v2022.3 (spirv-tools 2022.4, glslang r27-beta1).
- В NDK есть C-заголовки Vulkan 1.3.275 и stub `libvulkan.so` для API 24-35.
- В NDK **нет** `vulkan/vulkan.hpp` (Vulkan-Hpp) и **нет** `spirv/unified1/spirv.hpp`.
- Vulkan SDK на хосте не установлен, `VULKAN_SDK` не задан.
- MSVC BuildTools 2022 есть (`cl.exe` 19.44), но не в PATH.

**Что сделано (факт):**
1. Склонированы `Vulkan-Headers` и `SPIRV-Headers` в `build/llama-vulkan/toolchain` (сеть работает).
2. Сделан минимальный `SPIRV-HeadersConfig.cmake` (пакет нужен только для `find_package(CONFIG REQUIRED)`).
3. Собран `llama-bench` из `external/llama-clover-src` с NDK-тулчейном, NDK `libvulkan.so`, NDK `glslc`, Vulkan-Headers и MSVC как host-компилятором для `vulkan-shaders-gen`.
4. Результат: `build/llama-vulkan/build-clover/bin/llama-bench`, ELF64 AArch64, 168 MB. Конфигурация 4 s, сборка ~6 мин.

**Ключевой нюанс:** `find_package(Vulkan COMPONENTS glslc)` в CMake 4.4 работает, если задать `Vulkan_INCLUDE_DIR`, `Vulkan_LIBRARY`, `Vulkan_GLSLC_EXECUTABLE`. Но `SPIRV-Headers` обязателен, а host-сборка `vulkan-shaders-gen` требует полноценный MSVC: без `vcvars64` CMake подхватывает NDK clang и падает на `-lkernel32`. Ещё нюанс: NDK glslc 2022.3 не знает `GL_KHR_cooperative_matrix`, `GL_EXT_integer_dot_product`, bf16/fp8 — все feature-тесты выключились. Для 8 Gen 3/Elite это критично: нужен LunarG SDK 1.4.x (glslc 2025.x) или Docker `ghcr.io/snapdragon-toolchain/arm64-android:v0.8`.

**Точная команда (сокращённо):**
```powershell
# в cmd с vcvars64
cmake -S external\llama-clover-src -B build\llama-vulkan\build-clover -G Ninja `
 -DCMAKE_TOOLCHAIN_FILE=$NDK\build\cmake\android.toolchain.cmake `
 -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-28 `
 -DGGML_VULKAN=ON -DGGML_OPENCL=OFF -DGGML_OPENMP=OFF -DGGML_LLAMAFILE=OFF `
 -DBUILD_SHARED_LIBS=OFF -DLLAMA_CURL=OFF -DLLAMA_OPENSSL=OFF `
 -DVulkan_GLSLC_EXECUTABLE=$NDK\shader-tools\windows-x86_64\glslc.exe `
 -DVulkan_INCLUDE_DIR=build\llama-vulkan\toolchain\Vulkan-Headers\include `
 -DVulkan_LIBRARY=$NDK\toolchains\llvm\prebuilt\windows-x86_64\sysroot\usr\lib\aarch64-linux-android\28\libvulkan.so `
 -DSPIRV-Headers_DIR=build\llama-vulkan\toolchain\spirv-config `
 -DCMAKE_CXX_FLAGS="-Ibuild\llama-vulkan\toolchain\SPIRV-Headers\include" `
 -DHOST_C_COMPILER=cl.exe -DHOST_CXX_COMPILER=cl.exe
```
Для нормальной сборки под новые устройства ставить SDK не обязательно, но **лучше** взять готовый Snapdragon-Docker: там уже Hexagon SDK, OpenCL SDK, NDK и Vulkan SDK.

## 4. Матрица решений

| Критерий | 845 / Adreno 630 / 6 GB | 8 Gen 3 / Adreno 750 / 16 GB | 8 Elite / Adreno 830 / 24 GB |
|---|---|---|---|
| CPU decode/pp | **26.6 / 41.8 (факт)** | надо измерить; ориентир 20-30 tg на 3B Q4 | надо измерить; ориентир выше 8 Gen 3 |
| OpenCL | 2.23 / 3.05 (факт) — нет | 1.5B Q4_0: 24.1 / 252; 3B: 21.4 / 136 (факт, PR #14841) | 1B F16: ~22 / 199-275 (факт, PR #14987) |
| Vulkan | устройство видно, но 16-bit storage → отказ; патч → SIGSEGV (факт) | работает с патчами; драйвер падает (PR #29165) | vendor-драйвер abort (#29786); Turnip MoE 21-31.5 tg (факт) |
| Prefill | GPU не помогает (3.05) | OpenCL 136-252 — главный выигрыш | 199-275+; главный выигрыш |
| Decode | GPU хуже CPU | около CPU, не гарантирован; мерить | около/ниже CPU; NPU — главная надежда |
| MoE / Clover | 11.7 GB не влезает; 0.6 t/s, I/O-bound | 11.7 GB на грани 16 GB; потенциал 10-20 t/s, мерить | 24 GB комфортно; 15-25 t/s, мерить |
| Квант | Q4_K_M или Q4_0; repack не работает | OpenCL: Q4_0/Q6_K; Vulkan: Q4_K/IQ; NPU: Q4_0/Q8_0/MXFP4/K | NPU: Q4_0/MXFP4; Vulkan: Q4_K/TQ (TQ рискован) |
| Ожидаемый выигрыш | 0 | prefill 2-5x, decode 0.8-1.5x | prefill 3-6x, decode 1-2x/NPU |
| Стоимость / риск | высокая / бессмысленно | 1-2 недели / средний | 2-4 недели / высокий |

**Пояснения:**
- **845:** единственный разумный backend — CPU. Clover на 845 — не «медленная модель», а **storage-bound demo**: 11.69 GB файл, 0.6 t/s × ~333 MB/token ≈ 200 MB/s. GPU/NPU не меняют UFS/page cache.
- **8 Gen 3:** 16 GB — это минимум для Clover, и то на грани: 11.7 GB модель + Android 3-4 GB + KV 1-2 GB. Нужен маленький ctx, mmap и контроль LMK. GPU даёт prefill, decode — не факт. OpenCL поддерживает только Q4_0/Q6_K/F16/F32 (PR #10693), не Q4_K/IQ; Vulkan поддерживает больше, но драйвер 750 нестабилен.
- **8 Elite:** 24 GB снимает проблему RAM; 85 GB/s и Adreno 830 дают лучший prefill; для decode сначала мерить CPU (Oryon очень быстрый), потом NPU. Vendor Vulkan на 830 сейчас рискованный; Turnip/Hexagon — альтернативы.

## 5. NPU: Hexagon 685 и новые

**845 / Hexagon 685 = v65 (факт).** `ggml-hexagon` в этом дереве собирает только v73/v75/v79/v81 (в clover-src ещё v85). Версии v65 нет. QNN/Genie для 845 — устаревший SNPE-путь, без GGUF и без динамического MoE; Clover 35B туда не влезает. **Вердикт: не тратить время.**

**8 Gen 3 = v75, 8 Elite = v79 (факт).** Upstream `ggml-hexagon` — реальный backend:
- Docker `ghcr.io/snapdragon-toolchain/arm64-android:v0.8`, `-DGGML_HEXAGON=ON`;
- 3.5 GB виртуального адресного окна на сессию, layer-split по сессиям, dynamic map/unmap для моделей >3.5 GB;
- поддерживает Q4_0/Q8_0/MXFP4 и K-кванты (repack), а также `MUL_MAT_ID`/`TOP_K`, то есть MoE-операции;
- README даёт пример v79: Llama-1B Q4_0 — pp128 169.4, tg64 51.5 t/s. Для v75 цифры надо мерить.

**QNN/Genie:** имеет смысл для dense-моделей с фиксированными shape и конвертацией в QNN-граф. Для Clover с динамическим роутингом экспертов и кастомной архитектурой — нет: QNN не строит динамический MoE-граф, а конвертация 35B в статический граф бессмысленна. **Вердикт: QNN/Clover — не тратить; ggml-hexagon для dense/MoE на 8 Gen 3/Elite — исследовать после CPU-базлайна.**

## 6. Эксперимент на 845, который уже дал окончательный ответ

**Что сделано (факт):**
1. Собран `llama-bench` с Vulkan из `4838ffb` (168 MB, AArch64).
2. `--list-devices` — GPU виден.
3. `llama-bench -ngl 99 -p 8 -n 8` — отказ `does not support 16-bit storage`.
4. Патч fp32-only (PR #27723) — `SIGSEGV` в `ggml_vk_create_buffer`.

Этого достаточно: **Vulkan на 845 не работает и не будет без глубокой переделки драйвера/бэкенда.**

**Если всё же нужен полный протокол (для истории):**
```bash
adb push build/llama-vulkan/build-exp/bin/llama-bench /data/local/tmp/sllm-vk-exp/
adb shell "cd /data/local/tmp/sllm-vk-exp && \
  ./llama-bench -m /data/local/tmp/sllm/Qwen2.5-0.5B-Instruct-Q4_K_M.gguf \
  -ngl 99 -p 128 -n 32 -r 3 -t 4"
```
**Критерий успеха:** стабильно 10 минут, без мусора в greedy-выводе, **tg ≥ 1.2× CPU (≥32 tok/s)** и **pp ≥ 2× CPU (≥84 tok/s)**. **Критерий провала:** crash, wrong output или tg ≤ CPU. Ожидание: провал.

**Для Clover:** критерий — decode ≥1.2 t/s (2× от 0.6). Но арифметика I/O говорит, что без роста page-cache hit rate выше ~50% это невозможно; GPU тут ни при чём.

## 7. CPU-резерв: что реально можно выжать на 845

1. **Repack на 845 — миф.** `GGML_CPU_REPACK=ON` включён, но `ggml_cpu_has_dotprod()`/`matmul_int8()` ложны, а repack-пути (`Q4_0_4_4`, `Q4_0_8_8`, K-quant repack) гейтятся именно ими. Фактически repack не активируется.
2. **Q4_0 вместо Q4_K_M.** У Q4_0 проще деквант; на A75 это может дать 10-20% decode при небольшой потере качества. **Надо измерить.**
3. **Affinity.** `--cpu-mask 0xF0 --cpu-strict 1` (cpu4-7, A75) или `taskset -c 4-7`; `--prio 2`. Четыре потока не должны мигрировать на A55. **Надо измерить.**
4. **Prefill chunk.** `-b 256 -ub 128` против текущих 128/64; prefill может вырасти на 10-20%, но compute buffer вырастет. **Надо измерить.**
5. **KV q8_0 уже стоит.** q4_0 KV даст экономию трафика только на длинном ctx; для 0.5B на коротком ctx — почти ничего. **Мерить качество.**
6. **FP16 не включать:** уже измерено +1% pp / -3% tg, в шуме.
7. **Clover:** единственные реальные рычаги — prefetch (`madvise(MADV_WILLNEED)`/`pread`), per-layer expert profile, mmap vs pread, KV q4_0. Это I/O-оптимизация, не compute.
8. **Модель:** dense ≤1.5B Q4; MoE на 6 GB не имеет смысла.
9. **Speculative decoding:** на memory-bound 845 скорее добавит трафика, чем ускорит; не приоритет.

## Вывод

- **845:** CPU 4 потока — лучший и единственный практичный backend. Vulkan не работает (16-bit storage, патч сегфолтится), OpenCL в 12-14 раз медленнее, NPU v65 не поддерживается. Clover на 845 — storage-bound демо, GPU/NPU его не спасут. **Не тратить время.**
- **8 Gen 3 / 8 Elite:** GPU даёт прежде всего **prefill**; decode надо сравнивать с очень сильным CPU. OpenCL на Adreno 750/830 уже даёт 136-252 pp и ~20-24 tg на 1.5-3B Q4_0. Vulkan мощнее по возможностям (MoE, K/IQ/TQ), но драйверы нестабильны. NPU Hexagon v75/v79 — самый перспективный путь, особенно для MoE, но требует Docker/Hexagon SDK и отдельного проекта.
- **Clover:** 845 — не вариант; 8 Gen 3 16 GB — на грани; 8 Elite 24 GB — минимально комфортно. Сначала мерить CPU+RAM, потом NPU/OpenCL, Vulkan — в последнюю очередь.
- **Никаких обещаний буста без замера.**
