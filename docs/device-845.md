# Профиль устройства: Xiaomi MI 8 (Snapdragon 845 / 6 GB)

Измерено на живом устройстве 2026-11-07. Пустые поля — вопросы, а не «ноль».

## Идентификация

- [x] Модель телефона: **Xiaomi MI 8 (dipper)**
- [x] Прошивка / Android: **LineageOS 15, Android 15 (SDK 35), userdebug/eng, release-keys**
- [x] SoC: `sdm845` / `SDM845`
- [x] ABI: `arm64-v8a` (abilist: arm64-v8a, armeabi-v7a, armeabi)
- [x] Page size: **4096** (16 KB выравнивание всё равно включаем в сборке)
- [x] RAM: `MemTotal 5 766 584 kB` (~5.50 GiB), zram swap 2 291 884 kB
- [x] Свободно на `/data`: **102 GB** (всего 109 GB)
- [x] Root: **adb root работает** (`adbd` перезапущен как root, uid=0). Отдельного `su` нет.
      `ro.debuggable=1`, `ro.build.type=user`. Приложение остаётся в песочнице;
      для отладки используем `run-as` (debug APK) и `/data/local/tmp`.

## CPU

- [x] Ядра: 4× Kryo 385 Gold (Cortex-A75) @ **2 803 200 kHz** + 4× Kryo 385 Silver (Cortex-A55) @ **1 766 400 kHz**
- [x] `asimddp` (dotprod): **нет**
- [x] `i8mm`: **нет**
- [x] `fp16` (`asimdhp`): **есть**
- [x] `crc32`: **есть**
- [x] Governor: `schedutil`
- [x] BogoMIPS: 38.40

> Следствие: квантованные CPU-ядра ggml работают без dotprod. Это ожидаемо медленнее
> 8 Gen 3; нельзя переносить числа с новых телефонов.

## Память приложения

- [ ] `memoryClass` / `largeMemoryClass`: измерить на установленном APK
- [x] `availMem` в покое: **3 752 468 kB** (~3.58 GiB)
- [ ] PSS приложения после старта: измерить
- [ ] Сколько выдержал до `onTrimMemory`: измерить
- [x] `VmSize` процесса: см. телеметрию в приложении

## GPU / NPU

- [x] GPU: **Adreno 630 v2**
- [x] Частоты: 257 / 342 / 414 / 520 / 596 / 675 / **710 MHz**
- [x] Vulkan: `ro.hardware.vulkan=adreno`, `/system/lib64/libvulkan.so` есть
- [x] OpenCL: `/vendor/lib64/libOpenCL.so` есть
- [x] Hexagon 685 / HVX: есть (`compute-hvx` thermal zone), путь — QNN/Genie позже

## Накопитель

- [ ] Последовательное чтение, MB/s: измерить `dd` с `iflag=direct`
- [ ] Случайное 4K, IOPS: измерить
- [ ] Случайное 1–2 MB (эксперты), MB/s: измерить

## Тепло

- [x] Thermal zones: `cpu0-silver`, `cpu3-gold`, `gpu0/gpu1`, `ddr`, `compute-hvx`, ...
- [x] В покое: CPU ~33.5 °C, GPU ~30.4 °C, DDR ~30.7 °C
- [ ] 10 минут нагрузки: tok/s в начале / в конце
- [ ] Thermal status (idle / load)

## Baseline (CLI, CPU-only, NDK 28.2, llama.cpp f7591a6)

Условия: Qwen2.5-0.5B-Instruct Q4_K_M, `-c 2048 -t 4 --no-warmup`,
накопитель /data/local/tmp, экран включён, без зарядки.

| Приложение | Модель | TTFT | prefill tok/s | decode tok/s | Peak PSS | Thermal |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| llama-completion CLI (mainline) | Qwen2.5-0.5B Q4_K_M | 204 ms (8 tok) | **39.2** | **29.5** | TBD | 0 (none) |
| SnapdragonLLM app (release, 4838ffb) | Qwen2.5-0.5B Q4_K_M | **1303 ms** | **48.0** | **25.2** | ~0.53–0.61 GB | 0 (none) |
| SnapdragonLLM app (clover-pr 4838ffb) | Clover APEX-I-NanoPlus | **26 210 ms** (64 tok) | **2.44** | **0.60** | **~4.26 GB peak RSS** | 0 (none) |

Qwen app: ctx 4096, batch 128, KV q8_0, flash on, 4 потока. Ниже CLI, потому что
в процессе ещё UI/Compose и замер шёл после серии тяжёлых прогонов.
Clover: 24 токена, decode 66 s total; ответ осмысленный, но обрезан стопом.
KV cache в логе 0 MB — `llama_state_get_size` на этом контексте вернул 0;
пересчитать/заменить метрику.

## Факты, которые уже видны

- Android 15 на 4 KB pages. Сборка `arm64-v8a`, `android-28`, `-O3 -DNDEBUG`.
- Dotprod нет → CPU backend выбирает обычные NEON-ядра. Нужно проверить i8mm-репак
  и `Q4_0_4_4`/`Q4_0_8_8` — на A75 они могут дать выигрыш даже без dotprod.
- 3.58 GiB доступно в покое; бюджет приложения на 6 GB реалистично **2.5–3.2 GiB**.
- Clover 11.69 GB не влезает в RAM: только `mmap` + page cache + потоковая подгрузка
  экспертов. Скорость будет определяться UFS и hit rate.

## Вердикт

- [ ] Clover на 845 имеет смысл: да / нет / только demo.
- [x] Backend по умолчанию: **CPU (NEON)** до A/B Vulkan/OpenCL.
- [ ] Бюджет кэша, MB:
- [ ] Ограничения, которые выяснились:
