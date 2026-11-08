# 12 — Сборка и запуск

## Проверенный toolchain (2026-11-07)

| Компонент | Версия | Почему |
| --- | --- | --- |
| JDK | 21.0.11 (Eclipse Adoptium) | Gradle 8.14.3 + AGP 8.13.2 |
| Gradle | 8.14.3 | кэш уже есть; совместим с AGP 8.13.2 |
| AGP | 8.13.2 | последний стабильный 8.x |
| Kotlin | 2.3.0 | Compose compiler plugin той же версии |
| Compose BOM | 2025.10.01 | требует AGP 8.x / compileSdk 36; BOM 2026.x требует AGP 9.1 |
| compileSdk / targetSdk | 36 | Android 15/16 |
| minSdk | 28 | MI 8 / LineageOS 15 |
| NDK | 28.2.13676358 | 16 KB page support, clang 19 |
| CMake | 3.22.1 (SDK) | стабильно с NDK 28 |
| llama.cpp | f7591a6 + локальные патчи | clover_moe, MoE cache |

`local.properties`:

```properties
sdk.dir=C\:\\Users\\<user>\\AppData\\Local\\Android\\Sdk
```

## Команды

```powershell
# Забрать llama.cpp (один раз)
.\scripts\fetch-llama.ps1

# Debug APK (быстро, без R8)
.\scripts\build-apk.ps1

# Debug + install
.\scripts\build-apk.ps1 -Install

# Release (unsigned, нужен свой keystore для установки)
.\scripts\build-apk.ps1 -Variant release

# Локальная установка release: подпись debug-ключом (только для тестов)
.\scripts\sign-release.ps1
adb install -r app\build\outputs\apk\release\app-release-signed.apk
```

Или напрямую:

```powershell
.\gradlew.bat :app:assembleDebug --no-daemon
adb install -r app\build\outputs\apk\debug\app-debug.apk
```

## Нативное ядро

`app/src/main/cpp/CMakeLists.txt` делает `add_subdirectory(external/llama.cpp)` и
линкует `llama` + `llama-common` **статически** в один `libsnapcore.so`.

Флаги:

```
-DGGML_NATIVE=OFF        # кросс-сборка, без -march хоста
-DGGML_OPENMP=OFF        # нет libomp в NDK stable API
-DGGML_LLAMAFILE=OFF
-DGGML_BACKEND_DL=OFF    # без динамических backend-плагинов
-DGGML_CPU_KLEIDIAI=OFF  # submodule не нужен
-DGGML_CPU_ALL_VARIANTS=OFF
-DLLAMA_BUILD_EXAMPLES/TOOLS/SERVER=OFF
-DLLAMA_CURL=OFF
```

Плюс `-O3 -DNDEBUG` через `cppFlags`, чтобы debug-вариант не терял скорость.
Линковка: `-Wl,-z,max-page-size=16384` (Android 15+).

## Почему не готовый APK/библиотека

- Готовые сборки Clover на HF — x86_64 CPU/CUDA/Vulkan. Для `arm64-v8a` их нет.
- Поэтому ядро собирается из исходников под NDK; это же нужно для будущих
  патчей экспертного кэша и prefetch.

## Частые проблемы

| Симптом | Причина | Решение |
| --- | --- | --- |
| `Cannot find Java installation ... languageVersion=17` | `jvmToolchain(17)`, а JDK только 21 | убрать toolchain, использовать `compilerOptions.jvmTarget` |
| `Dependency ... requires AGP 9.1` | слишком новый Compose BOM | BOM 2025.10.01 |
| `library "libllama-...so" not found` (CLI) | запуск без `LD_LIBRARY_PATH` | `LD_LIBRARY_PATH=. ./llama-completion` |
| `key not found: clover_moe.expert_shared_count` | старый GGUF без ключа | патч загрузчика (см. `13-CLOVER.md`) |
| `wrong number of tensors: expected 853, got 733` | block-diffusion тензоры не читаются | патч загрузчика (см. `13-CLOVER.md`) |
| `adb push` в `/sdcard/Android/data/...` запрещён | нет root/Android 11 | на этом устройстве `adb root` работает; иначе SAF-импорт в приложении |

## Проверка APK

```powershell
# 16 KB alignment всех .so
& "$env:ANDROID_HOME\build-tools\36.0.0\zipalign.exe" -c -P 16 -v 4 app\build\outputs\apk\debug\app-debug.apk

# Какие ABI внутри
& "$env:ANDROID_HOME\build-tools\36.0.0\aapt.exe" dump badging app\build\outputs\apk\debug\app-debug.apk | Select-String "native-code"
```
