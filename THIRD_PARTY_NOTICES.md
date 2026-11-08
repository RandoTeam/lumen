# Third-party notices

This project links and redistributes the following components.

## llama.cpp / ggml

- Source: <https://github.com/ggml-org/llama.cpp>
- License: MIT
- Copyright: ggml-org and llama.cpp contributors.
- The APK uses a **modified** checkout at commit
  `4838ffb0c8ade92f34d9157deea38d92758ab824` (PR #31306). Modifications and
  reasons are documented in `docs/13-CLOVER.md`.
- The MIT notice must be kept in binary distributions.

## Clover-35B-A3B-Preview

- Base model: DiffuSpace, Apache-2.0.
- Quantization: IsValorum, APEX-I-NanoPlus V2.1.
- The GGUF is **not** shipped in the APK; the user imports it.
- Attribution: DiffuSpace / IsValorum.

## AndroidX / Jetpack Compose

- License: Apache-2.0.
- Copyright: The Android Open Source Project.

## Khronos OpenCL-Headers

- Used only for the experimental OpenCL build under `external/OpenCL-Headers`.
- License: Apache-2.0.
- Not part of the shipped CPU-only APK.

## Android NDK / clang runtime

- The APK contains code compiled with the Android NDK toolchain.
- Follow the Android NDK license terms for redistribution.

## Ternary-Bonsai 2-4B and Uncensored LoRA (planned/test)

- Base: `prism-ml/Ternary-Bonsai-2-4B-gguf`, Apache-2.0.
- Upstream base: Qwen3.5-4B, Alibaba, Apache-2.0.
- LoRA: `AtomicChat/Ternary-Bonsai-2-4B-Uncensored-LoRA-GGUF`, Apache-2.0,
  produced by AtomicChat on top of Prism ML Bonsai.
- Keep `NOTICE.txt` and `LICENSE` from both repositories.
- The uncensored adapter removes learned refusals. It is research-grade:
  age-gate it, keep it off by default, show a disclaimer, and allow the user to
  disable it. Do not ship it in a child-directed profile.

## Planned multimodal components

| Component | License | Role |
| --- | --- | --- |
| llama.cpp `mtmd` / `libmtmd` | MIT | image/audio/video input |
| SmolVLM / SmolVLM2 | Apache-2.0 | default vision/video on 6 GB |
| Qwen3-ASR | Apache-2.0 | default ASR |
| whisper.cpp / Whisper | MIT | lite ASR fallback |
| stable-diffusion.cpp | MIT | Studio image generation |
| SD1.5 | CreativeML OpenRAIL-M | use restrictions apply |
| SD-Turbo | non-commercial (verify) | demo only |
| Android `TextToSpeech` | system API | voices may be proprietary |
| MediaCodec / MediaMetadataRetriever | system API | video frames |

These are not all shipped in the current CPU-only APK. Add the notices before
enabling each package.

## Project code

No license has been chosen for the application code in this repository yet.
Pick one before publishing. Model and library licenses above still apply.
