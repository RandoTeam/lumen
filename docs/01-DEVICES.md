# 01 — Устройства и бюджет памяти

## Матрица

| | Snapdragon 845 (ворота) | Snapdragon 8 Gen 3 (Pad 2) | Snapdragon 8 Elite (OnePlus 13) |
| --- | --- | --- | --- |
| Устройство | телефон, 6 GB | OnePlus Pad 2, 16/512 (OPD2403) | OnePlus 13, 24 GB |
| Техпроцесс | 10 nm LPP | 4 nm | 3 nm |
| CPU | 4× Kryo 385 Gold (A75) до 2.8 GHz + 4× Kryo 385 Silver (A55) 1.8 GHz | 1× X4 3.3 + 3× A720 3.15 + 2× A720 2.96 + 2× A520 2.27 GHz | 2× Oryon 4.32 + 6× Oryon 3.53 GHz |
| GPU | Adreno 630 | Adreno 750 | Adreno 830 |
| DSP/NPU | Hexagon 685 (HVX) | Hexagon NPU | Hexagon NPU |
| Память | LPDDR4X, 2 канала, ~30 GB/s теоретически | LPDDR5X, ~77 GB/s теоретически | LPDDR5X, ~80–85 GB/s теоретически |
| Накопитель | UFS 2.1 | UFS 4.0 | UFS 4.0 |
| GPU API | OpenCL 2.0, Vulkan 1.1, GLES 3.2 | OpenCL 3.0, Vulkan 1.3 | OpenCL 3.0, Vulkan 1.3 |
| Android на старте | 8.0; на устройстве обычно 9–12 | 14 / OxygenOS 14 | 15 / OxygenOS 15 |

Точные частоты и пропускную способность **проверить на своём экземпляре**: прошивки,
режимы энергосбережения и thermal throttling меняют цифры. Таблица выше — паспорт, не
результат измерения.

## Главное отличие от 4 GB CUDA-проекта

Там было:

```
4 GB VRAM (отдельная)  +  32 GB RAM  +  SSD
```

Здесь:

```
одна физическая RAM (6 / 16 / 24 GB)
   ├─ GPU-видимая часть (Adreno, OpenCL/Vulkan buffers)
   ├─ CPU-часть (обычные malloc/mmap)
   └─ page cache ОС (файлы моделей)
        └─ SSD (UFS)
```

Слова «VRAM» и «RAM» на Android — это **режим доступа**, а не два разных куска железа.
Когда мы говорим «держать эксперта в GPU-кэше», мы отнимаем эти байты у системы и CPU.
Поэтому бюджет один, и его надо делить явно.

## Бюджет памяти: как считать

Номинал (6/16/24 GB) — не бюджет. Реальный бюджет:

```
usable = min(
    ActivityManager.memoryClass,       // сколько ОС готова дать процессу
    ActivityManager.MemoryInfo.availMem,
    nominal - system_reserve
) - safety_reserve
```

`system_reserve` на 6 GB ориентировочно 2.5–3.5 GB (ОС, лаунчер, фоновые приложения), на
16–24 GB — 4–8 GB. Это **гипотеза**, её надо измерить (см. Phase 0).

Дальше:

```
usable
 - code + JVM + UI + native libs
 - KV cache (зависит от ctx, слоёв, голов, типа KV)
 - dense weights (attention, shared expert, embeddings, head)
 - prefill/scratch buffers
 - GPU driver overhead
 = бюджет на экспертный кэш и page cache
```

На 6 GB реалистично рассчитывать на **3–3.5 GB** на всё приложение; на 16 GB — на 8–12 GB;
на 24 GB — на 12–18 GB. Проверять, не верить.

### Что занимает Clover

| Компонент | Оценка | Комментарий |
| --- | ---: | --- |
| Файл модели | 11.69 GB | mmap с SSD, не копируется в RAM целиком |
| Dense-часть | ~1.3–1.7 GB | attention/GDN, shared expert, embeddings, head; зависит от кванта |
| KV cache | 0.1–1 GB | 2K–32K контекст, q8_0/q4_0 |
| Экспертный кэш | остаток | 10 240 экспертов; влезет несколько сотен–тысяч |
| Page cache | сколько даст ОС | холодные эксперты читаются с UFS |

Цифры dense/KV — из 4 GB проекта и подлежат пересчёту на Android.

## Snapdragon 845: что именно проверять

1. **CPU-инструкции.** `cat /proc/cpuinfo` → есть ли `asimddp` (dot product), `fp16`,
   `crc32`. Cortex-A75 — ARMv8.2-A, но набор расширений зависит от bin'а. Если dotprod
   нет, квантованные CPU-ядра будут заметно медленнее, чем на 8 Gen 3.
2. **GPU-драйвер.** `adb shell dumpsys SurfaceFlinger` / GPU info; проверить, что
   OpenCL-устройство видно (`clinfo`-подобный тест) и Vulkan 1.1 работает.
3. **Доступная память.** `dumpsys meminfo <pkg>`, `cat /proc/meminfo`,
   `ActivityManager.getMemoryInfo`.
4. **Накопитель.** Последовательное и случайное чтение блоками 1–2 MB (эксперты) и 4 KB.
5. **Тепло.** 10–30 минут нагрузки, `dumpsys thermalservice`, температура батареи,
   частота CPU/GPU.
6. **Android и ABI.** Только `arm64-v8a`; проверить 16 KB page size, если Android 15+.

## 8 Gen 3 / 8 Elite: что будет важно

- Больше RAM → можно держать больше экспертов и длинный контекст.
- Другой GPU → другие оптимальные кванты (Q4_0/Q8_0 на GPU, K-quants на CPU).
- NPU (Hexagon) есть, но путь к нему — через Qualcomm QNN/Genie и конвертацию модели;
  это отдельная ветка, не замена llama.cpp.
- UFS 4.0 быстрее, но случайное чтение всё равно на порядки медленнее RAM; кэш всё равно
  нужен.

## Ограничения Android, которые влияют на архитектуру

- **Low Memory Killer.** Большой mmap и большой native heap могут привести к убийству
  процесса. Нужен `onTrimMemory` и явное освобождение GPU-буферов.
- **Foreground service.** Долгий inference — только foreground service с уведомлением;
  на Android 14+ объявить `foregroundServiceType` (`specialUse` или `dataSync`).
- **16 KB page size.** Для Android 15+ нативные библиотеки и буферы должны быть
  выровнены на 16 KB; собирать NDK r28+ и проверять `zipalign -P 16`.
- **Scoped storage.** Модель 11.69 GB не кладём в APK; скачиваем/импортируем через
  Storage Access Framework или app-specific external files.
- **Батарея и тепло.** Длительный decode упирается в throttling; замеры — после прогрева
  и с записью thermal status.
- **Фоновые ограничения.** Экран выключен → меньше ресурсов; для бенчмарка фиксируем
  состояние (экран включён, яркость 50%, авиарежим).

## Что считать успехом на каждом устройстве

- **845 / 6 GB:** приложение стабильно запускает Clover, не падает по OOM, выдаёт
  осмысленный ответ; скорость — любая измеренная, без обещаний.
- **Pad 2 / 16 GB:** Clover работает с приемлемым TTFT и устойчивой скоростью; кэш
  экспертов заметно больше.
- **OnePlus 13 / 24 GB:** максимум, который даёт железо; сравнение с 16 GB показывает
  вклад RAM, а не только SoC.

Точные пороги по tok/s появятся после Phase 0. До этого места в документах, где стоит
«TBD», — не забытые числа, а сознательно не выдуманные.
