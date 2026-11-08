# Отчёт 11. Полностью Rust APK: Android packaging

Источник: практический прототип, 2026-11-08. Проверено на живом MI 8 (Android 15).

## TL;DR

- **UI + вся логика — на Rust, без Kotlin.** Slint 1.18.1 +
  `backend-android-activity-06` + `android.app.NativeActivity`, упаковка через
  **cargo-apk2 1.5.0 без Gradle**. Проверено: APK собирается, ставится, рисует,
  принимает тапы, открывает IME (русская клавиатура), переживает home/resume.
- **Release APK (arm64, software renderer, stripped): 8.48 МБ.** Debug: 48.8 МБ.
  16 KB pages подтверждены (`LOAD ... 0x4000`, `zipalign -c -P 16` — OK).
- **Два места, где нужен тонкий Java-shim (~120 строк, не Kotlin):**
  1. **Foreground service** — класс-наследник `android.app.Service`, иначе
     система не сможет его инстанцировать.
  2. **SAF-импорт** — `android-activity` 0.6.1 не пробрасывает
     `onActivityResult`; пикер надо запускать из своего Java-класса (наследник
     `NativeActivity`) и звать `nativeOnActivityResult`.
- Остальное — notifications, intents, permissions, JNI, файлы, SHA-256,
  wake lock — из Rust через `jni`/`ndk`.
- **`cargo-apk2` компилирует Java-исходники в dex** (`java_sources`), это
  легальный путь «Rust-проект + 2 Java-файла, без Gradle».

## 1. Рецепт проекта (проверенный)

`C:\Temp\slint-android-probe`:

```
Cargo.toml
build.rs
ui/app.slint
src/lib.rs
java/  (позже: SyncService.java, MainActivity.java — shim)
```

**Cargo.toml**

```toml
[package]
name = "slintprobe"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
log = "0.4"
android_logger = "0.15"

[target.'cfg(target_os = "android")'.dependencies]
slint = { version = "1.18", default-features = false,
          features = ["compat-1-2", "backend-android-activity-06",
                      "renderer-software", "std"] }
android-activity = { version = "0.6", features = ["native-activity"] }

[build-dependencies]
slint-build = "1.18"

[package.metadata.android]
use_aapt2 = true
package = "dev.probe.slint"
apk_name = "slintprobe"
build_targets = ["aarch64-linux-android"]
strip = "strip"
java_sources = "java"

[package.metadata.android.sdk]
min_sdk_version = 26
target_sdk_version = 36

[[package.metadata.android.application.activity]]
name = "android.app.NativeActivity"

[[package.metadata.android.application.activity.meta_data]]
name = "android.app.lib_name"
value = "slintprobe"

[[package.metadata.android.application.activity.intent_filter]]
actions = ["android.intent.action.MAIN"]
categories = ["android.intent.category.LAUNCHER"]

[package.metadata.android.signing.release]
path = 'C:\Users\Ilia V\.android\debug.keystore'
keystore_password = 'android'
```

**build.rs**

```rust
fn main() { slint_build::compile("ui/app.slint").expect("slint compile"); }
```

**src/lib.rs**

```rust
#![cfg(target_os = "android")]
use slint::android::android_activity::{MainEvent, PollEvent};
slint::include_modules!();

#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    android_logger::init_once(android_logger::Config::default()
        .with_max_level(log::LevelFilter::Info).with_tag("slintprobe"));
    slint::android::init_with_event_listener(app, |ev| {
        if let PollEvent::Main(m) = ev { log::info!("lifecycle: {m:?}"); }
    }).unwrap();
    MainWindow::new().unwrap().run().unwrap();
}
```

**ui/app.slint** — ключевое: `root.safe-area-insets.*`.

```slint
import { Button, VerticalBox, LineEdit } from "std-widgets.slint";
export component MainWindow inherits Window {
    in-out property <int> counter: 0;
    VerticalBox {
        padding-left: root.safe-area-insets.left + 24px;
        padding-right: root.safe-area-insets.right + 24px;
        padding-top: root.safe-area-insets.top + 24px;
        padding-bottom: root.safe-area-insets.bottom + 24px;
        Text { text: "SnapdragonLLM / Slint probe"; color: white; }
        name := LineEdit { placeholder-text: "type here (IME test)"; }
        Text { text: name.text == "" ? "…" : "Hello, " + name.text; }
        Button { text: "Tapped " + root.counter + " times"; clicked => { root.counter += 1; } }
        Text { text: "size: " + (root.width / 1px) + "x" + (root.height / 1px); }
    }
}
```

`renderer-software` — быстрый старт; для продакшена `renderer-skia` (лучший
текст/анимации, +10–15 МБ на ABI).

## 2. Сборка APK

```powershell
cargo install cargo-apk2
$env:ANDROID_HOME="C:\Users\Ilia V\AppData\Local\Android\Sdk"
$env:ANDROID_NDK_ROOT="$env:ANDROID_HOME\ndk\28.2.13676358"
$env:JAVA_HOME="C:\Program Files\Eclipse Adoptium\jdk-21.0.11.10-hotspot"
$env:PATH="$env:JAVA_HOME\bin;$env:PATH"
cargo apk2 build --release --target aarch64-linux-android
```

Результаты: release **8 475 103 B**; debug 48.8 MB; `classes.dex` 23 248 B
после подключения Java-хелпера Slint (до него 604 B, IME не работал).

Проверка 16 KB:

```powershell
zipalign -c -P 16 -v 4 target\debug\apk\slintprobe.apk
llvm-readelf -lW target\aarch64-linux-android\debug\libslintprobe.so | Select-String LOAD
```

**Ошибки, которые встретились:**

1. `Cannot convert length to string. Divide by 1px` — в Slint нельзя `"" + width`.
2. `Configure a release keystore` — release без keystore не подписывается.
3. `UnsupportedClassVersionError: D8 class file 55.0` — d8 из build-tools 36
   требует JDK 11+; `JAVA_HOME` недостаточно, cargo-apk2 зовёт `java` из PATH.
4. `can't find crate for core` — транзиентный сбой rustup; повторная сборка.
5. `linker cc not found` при `cargo build --target` — нужен `cargo apk2`/`cargo ndk`.
6. `java_sources` должен быть внутри `[package.metadata.android]`, иначе молча
   уходит в `signing.release`.

Альтернативы: `cargo-ndk` + Gradle (AGP 8.7+, JDK 17/21) — для продакшена
(ABI splits, AAB, signing); `xbuild` — git master; старый `cargo-apk 0.10` — не
трогать.

## 3. Android API из Rust: что без shim

| Задача | Как | Shim? |
|---|---|---|
| Activity/Context/PackageManager | `app.activity_as_ptr()`, `app.vm_as_ptr()` + `jni 0.22` | нет |
| Permissions | `checkSelfPermission/requestPermissions` через JNI | да* |
| Notifications + channel | JNI `NotificationChannel`, `Notification.Builder`, `notify` | нет |
| Wake lock | JNI `PowerManager.newWakeLock` | нет |
| Intents | JNI `Intent`, `startActivity` | нет |
| **SAF/пикер файла** | `onActivityResult` не форвардится | **да** |
| **Foreground service** | нужен класс-наследник `Service` | **да** |
| IME/insets/predictive back | Slint Java-хелпер `SlintAndroidJavaHelper.java` | **обязательно** |

\* `requestPermissions` результат приходит в `onRequestPermissionsResult`,
которого `android-activity` не форвардит → через Java-шим.

**Критично:** без `java_sources` на папку
`.../i-slint-backend-android-activity-1.18.1/java` приложение запускается, но
`safe-area-insets == 0`, IME не открывается, back не перехватывается. После
подключения тап по `LineEdit` → `mInputShown=true`, клавиатура и выделение
работают.

## 4. Lifecycle (измерено на MI 8)

Старт: `android_main → Start → Resume → InitWindow → WindowResized →
ContentRectChanged → RedrawNeeded → GainedFocus`; `Displayed +645 ms`,
PSS ≈ 55 MB (native heap 6.9 MB, `.so` mmap 18 MB) — software renderer.

Home → `Pause → LostFocus → TerminateWindow → Stop → SaveState`; возврат в тот же
процесс: `Start → Resume → InitWindow → ...` (Activity не пересоздаётся).

Следствия:
- `android_main` может вызываться повторно; глобальное состояние — в
  `OnceLock`/`static`, не в замыкании Activity.
- Ориентация/resize приходят как `WindowResized`/`ConfigChanged`/
  `ContentRectChanged`, без recreate. Это нужно Pad 2.
- IME: окно не ресайзится (edge-to-edge), клавиатура — нижний inset в
  `safe-area-insets`. Поле ввода прибивать к `root.safe-area-insets.bottom`.
- Predictive back: back при клавиатуре закрывает IME; при закрытой — `Destroy`.
  Для навигации нужен явный перехват/кнопка.
- Мультиокно: проверять `content_rect`, не хардкодить 2120×3000.

## 5. Foreground service (обязательный Java-shim)

`java/dev/snapdragonllm/GenerationService.java` (~70 строк):

```java
package dev.snapdragonllm;
public class GenerationService extends android.app.Service {
    public static native void nativeStart();
    public static native void nativeStop();
    @Override public int onStartCommand(android.content.Intent i, int f, int id) {
        String a = i == null ? null : i.getAction();
        if ("STOP".equals(a)) { nativeStop(); stopForeground(STOP_FOREGROUND_REMOVE); stopSelf(); return START_NOT_STICKY; }
        startForeground(1, buildNotification());
        nativeStart(); return START_STICKY;
    }
    @Override public android.os.IBinder onBind(android.content.Intent i) { return null; }
    private android.app.Notification buildNotification() { /* Notification.Builder(this, "sync") ... */ }
}
```

Манифест (cargo-apk2):

```toml
[[package.metadata.android.application.service]]
name = ".GenerationService"
foreground_service_type = "dataSync"
[[package.metadata.android.uses_permission]]
name = "android.permission.FOREGROUND_SERVICE"
[[package.metadata.android.uses_permission]]
name = "android.permission.FOREGROUND_SERVICE_DATA_SYNC"
[[package.metadata.android.uses_permission]]
name = "android.permission.POST_NOTIFICATIONS"
```

На Android 14+ `dataSync` имеет лимит ~6 ч/24 ч; для длинного inference —
`specialUse` + property, но это ревью Play. Channel создавать из Rust, иначе
уведомление молча не покажется.

## 6. Файлы и логи

- `app.internal_data_path()` / `app.external_data_path()` — без разрешений,
  видны по USB.
- Импорт: `ContentResolver.openInputStream(uri)` → `copy` кусками 64–256 КБ в
  `<dir>/name.part`, параллельно SHA-256; `sync_all`, сверка, `rename`.
- Докачка `content://` Range не работает; при обрыве начинать заново.
- Место: `statvfs`; резерв ≥1.5 GB; отказ до копирования.
- `takePersistableUriPermission` — в Java-шиме.
- 12 GB > 4 GB: везде `u64`, только arm64.
- Логи: `android_logger` + файл в `filesDir/logs/app.log`.

## 7. Тестирование

```powershell
adb install -r -t target\debug\apk\slintprobe.apk
adb shell am start -W -n dev.probe.slint/android.app.NativeActivity
adb logcat -s slintprobe:V *:S
adb shell dumpsys meminfo dev.probe.slint
adb shell input tap 500 1215
adb shell input tap 500 1004
adb shell dumpsys input_method | Select-String mInputShown
```

MI 8: cold start, IME RU/EN, home/resume, back, PSS при 6 GB.
Pad 2: split-screen/windowing, resize, insets, predictive back, 16 KB pages.

## 8. Риски и рекомендация

**Рецепт:** `cargo-apk2` + Slint 1.18 (`backend-android-activity-06`) +
`java_sources` (Slint helper) + 2 Java-класса (NativeActivity-сабкласс для
SAF/разрешений, Service для FGS) + `runtime_libs`/статическая линковка
llama.cpp. Kotlin — 0 строк.

**Риски:**
- `cargo-apk2` — один мейнтейнер; нет ABI splits/AAB/Play-подписи. Для релиза
  держать Gradle-путь (`cargo ndk` + AGP).
- Slint-хелпер — Java в APK: `hasCode=true`, иначе IME/insets/back ломаются.
- SAF/FGS без Java невозможны.
- llama.cpp: несколько `.so` или статическая линковка; проверить
  `LOAD align 0x4000` у всех библиотек.
- `renderer-software` — 8.5 MB, но слабее текст/анимации; Skia +12 MB.
- IME не ресайзит окно; вся вёрстка на `safe-area-insets`.
