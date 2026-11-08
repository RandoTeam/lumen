# 09 — Ссылки

## Clover

| Что | Ссылка |
| --- | --- |
| Базовая модель (DiffuSpace) | <https://huggingface.co/Diffuspace/Clover-35B-A3B-Preview> |
| Блог DiffuSpace | <https://huggingface.co/blog/Diffuspace/clover-35b-a3b-preview> |
| GGUF от DiffuSpace | <https://huggingface.co/Diffuspace/Clover-35B-A3B-Preview-GGUF> |
| Наш квант APEX-I-NanoPlus V2.1 (IsValorum) | <https://huggingface.co/IsValorum/Clover-35B-A3B-Preview-APEX-I-NanoPlus-V2.1-GGUF> |
| Q3_K_M / Q4_K_M (maternion) | <https://huggingface.co/maternion/Clover-35B-A3B-Preview-GGUF> |
| Uncensored (AtomicChat) | <https://huggingface.co/AtomicChat/Clover-35B-A3B-Preview-Uncensored-GGUF> |
| Clover runtime (IsValorum, prebuilt + converter) | <https://huggingface.co/IsValorum/llama.cpp-clover-moe> |
| PR #31306 (clover_moe runtime, не merged) | <https://github.com/ggml-org/llama.cpp/pull/31306> |
| Clover config (HF) | <https://huggingface.co/Diffuspace/Clover-35B-A3B-Preview/blob/main/config.json> |
| PolyStrata notes on Clover | <https://github.com/simplexai-labs/PolyStrata/blob/main/docs/models/clover.md> |

## Движки и библиотеки

| Проект | Роль | Ссылка |
| --- | --- | --- |
| llama.cpp | **основа ядра**: GGUF, MoE, K/IQ/TQ, CPU/GPU | <https://github.com/ggml-org/llama.cpp> |
| ggml | тензорная библиотека под llama.cpp | <https://github.com/ggml-org/ggml> |
| llama.cpp Android | сборка под Android | <https://github.com/ggml-org/llama.cpp/blob/master/docs/android.md> |
| llama.cpp OpenCL (Adreno) | GPU-бэкенд Qualcomm | <https://github.com/ggml-org/llama.cpp/tree/master/ggml/src/ggml-opencl> |
| llama.cpp Vulkan | GPU-бэкенд | <https://github.com/ggml-org/llama.cpp/tree/master/ggml/src/ggml-vulkan> |
| PocketPal AI | UX-референс, baseline (Flutter + llama.cpp) | <https://github.com/a-ghorbani/pocketpal-ai> |
| Google AI Edge Gallery | референс нативного приложения и бенчмарков | <https://github.com/google-ai-edge/gallery> |
| LiteRT-LM | inference-стек Google для edge | <https://github.com/google-ai-edge/LiteRT-LM> |
| MLC-LLM | research: GPU-стек TVM/Adreno | <https://github.com/mlc-ai/mlc-llm> |
| MNN | research: Android CPU/GPU | <https://github.com/alibaba/MNN> |
| ExecuTorch | research: PyTorch Edge | <https://github.com/pytorch/executorch> |
| Qualcomm QNN / Genie | NPU Hexagon (позже) | <https://www.qualcomm.com/developer/software/qualcomm-ai-engine-direct> |

## Тернарные модели

| Что | Ссылка |
| --- | --- |
| BitNet b1.58 (статья) | <https://arxiv.org/abs/2402.17764> |
| bitnet.cpp (Microsoft) | <https://github.com/microsoft/BitNet> |
| BitNet b1.58 2B4T | <https://huggingface.co/microsoft/bitnet-b1.58-2B-4T> |
| ggml TQ1_0/TQ2_0 | в исходниках ggml/llama.cpp |

## Android

| Тема | Ссылка |
| --- | --- |
| Управление памятью и `onTrimMemory` | <https://developer.android.com/topic/performance/memory> |
| Foreground services | <https://developer.android.com/develop/background-work/services/fgs> |
| 16 KB page size | <https://developer.android.com/guide/practices/page-sizes> |
| Thermal API | <https://developer.android.com/reference/android/os/PowerManager#getCurrentThermalStatus()> |
| Scoped storage | <https://developer.android.com/training/data-storage> |
| `dumpsys meminfo` | <https://developer.android.com/tools/dumpsys> |

## Железо

| Что | Ссылка |
| --- | --- |
| Snapdragon 845 | <https://www.qualcomm.com/smartphones/products/8-series/snapdragon-845-mobile-platform> |
| Snapdragon 8 Gen 3 | <https://www.qualcomm.com/smartphones/products/8-series/snapdragon-8-gen-3-mobile-platform> |
| Snapdragon 8 Elite | <https://www.qualcomm.com/smartphones/products/8-series/snapdragon-8-elite-mobile-platform> |
| OnePlus Pad 2 | <https://www.oneplus.com/us/oneplus-pad-2/specs> |
| OnePlus 13 | <https://www.oneplus.com/oneplus-13/specs> |

## Наш 4 GB проект (идеи, не доказательства для Android)

```
C:\VietnAi\4gbVramStrata
```

| Документ | Что взять |
| --- | --- |
| `docs/HOW_IT_WORKS_MODELS.md` | как устроен Clover и экспертный кэш |
| `docs/BENCH-4GB.md` | протокол измерений, примеры чисел |
| `docs/QUALITY-4GB.md` | 22-промптный gate, поведение моделей |
| `docs/MODELS-4GB.md` | карточки моделей и требования |
| `AGENTS.md` | правила проекта: не публиковать без gate, не переписывать числа |

Копии трёх документов лежат в `reference/from-4gbVramStrata/` с предупреждением:
**числа оттуда сняты на CUDA-ноутбуке с отдельной VRAM и не являются evidence для
Snapdragon.**
