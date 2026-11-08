//! Build script for `sllm-llama-sys`.
//!
//! Configures CMake against the pinned llama.cpp fork, builds the static
//! libraries plus the C++ shim, then runs bindgen over the public headers.
//!
//! Environment:
//! - `SLLM_LLAMA_SRC`      llama.cpp source root (default `../../external/llama-clover-src`)
//! - `SLLM_SYS_SKIP_BUILD` skip CMake/bindgen, compile the stub API instead
//! - `ANDROID_NDK_HOME`    required for Android targets
//! - `SLLM_CPU_VARIANT`    Android CPU baseline: baseline|v82|v86|v90
//! - `SLLM_OPENCL`         build the OpenCL backend (also `--features opencl`)
//! - `SLLM_SHARED`         build shared llama.cpp libraries instead of static
//! - `SLLM_CMAKE_GENERATOR` override the CMake generator
//! - `SLLM_CMAKE_PROFILE`  native build type (default `Release`)
//! - `LIBCLANG_PATH`       directory holding libclang.dll/.so/.dylib

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=CMakeLists.txt");
    println!("cargo:rerun-if-changed=include/sllm_shim.h");
    println!("cargo:rerun-if-changed=include/sllm_bindgen.h");
    println!("cargo:rerun-if-changed=src/sllm_shim.cpp");
    println!("cargo:rerun-if-env-changed=SLLM_LLAMA_SRC");
    println!("cargo:rerun-if-env-changed=SLLM_SYS_SKIP_BUILD");
    println!("cargo:rerun-if-env-changed=SLLM_CPU_VARIANT");
    println!("cargo:rerun-if-env-changed=SLLM_OPENCL");
    println!("cargo:rerun-if-env-changed=SLLM_SHARED");
    println!("cargo:rerun-if-env-changed=SLLM_CMAKE_PROFILE");
    println!("cargo:rerun-if-env-changed=SLLM_CMAKE_GENERATOR");
    println!("cargo:rerun-if-env-changed=LIBCLANG_PATH");
    println!("cargo:rerun-if-env-changed=ANDROID_NDK_HOME");
    println!("cargo:rerun-if-env-changed=ANDROID_NDK_ROOT");
    println!("cargo:rustc-check-cfg=cfg(sllm_sys_skip_build)");
    println!("cargo:rustc-check-cfg=cfg(sllm_android)");

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    if env_flag("SLLM_SYS_SKIP_BUILD") {
        println!(
            "cargo:warning=SLLM_SYS_SKIP_BUILD=1: not building llama.cpp; \
             the crate exposes a stub API that returns Error::NotBuilt"
        );
        println!("cargo:rustc-cfg=sllm_sys_skip_build");
        return;
    }

    let src = resolve_source(&manifest);
    require_file(&src.join("include/llama.h"), "llama.h");

    let android = target_os == "android";
    if android {
        println!("cargo:rustc-cfg=sllm_android");
    }

    let shared = env_flag("SLLM_SHARED");
    let opencl = env_flag("SLLM_OPENCL") || env::var_os("CARGO_FEATURE_OPENCL").is_some();

    let mut cfg = cmake::Config::new(&manifest);
    // Rust never links the MSVC debug CRT, so building llama.cpp with
    // CMAKE_BUILD_TYPE=Debug (/MDd) fails at link time. Always build the native
    // side optimised; the Rust profile stays independent.
    let cmake_profile = env::var("SLLM_CMAKE_PROFILE").unwrap_or_else(|_| "Release".into());
    cfg.profile(&cmake_profile);
    cfg.define("SLLM_LLAMA_SRC", &src)
        .define("BUILD_SHARED_LIBS", if shared { "ON" } else { "OFF" })
        .define("GGML_NATIVE", "OFF")
        .define("GGML_OPENMP", "OFF")
        .define("GGML_LLAMAFILE", "OFF")
        .define("GGML_CPU_KLEIDIAI", "OFF")
        .define("LLAMA_BUILD_COMMON", "ON")
        .define("LLAMA_BUILD_MTMD", "ON")
        .define("LLAMA_BUILD_TOOLS", "OFF")
        .define("LLAMA_BUILD_TESTS", "OFF")
        .define("LLAMA_BUILD_SERVER", "OFF")
        .define("LLAMA_BUILD_EXAMPLES", "OFF")
        .define("LLAMA_BUILD_APP", "OFF")
        .define("LLAMA_BUILD_UI", "OFF")
        .define("LLAMA_SUBPROCESS", "OFF")
        .define("LLAMA_CURL", "OFF")
        .define("LLAMA_OPENSSL", "OFF");

    if let Ok(generator) = env::var("SLLM_CMAKE_GENERATOR") {
        cfg.generator(generator);
    } else if android {
        cfg.generator("Ninja");
    }

    if android {
        configure_android(&mut cfg, &src, &target_arch, &manifest);
    } else {
        configure_host(&mut cfg, &src, &manifest);
    }

    if opencl {
        cfg.define("GGML_OPENCL", "ON")
            .define("GGML_OPENCL_USE_ADRENO_KERNELS", "ON");
        if let Ok(dir) = env::var("OPENCL_INCLUDE_DIR") {
            cfg.define("OpenCL_INCLUDE_DIR", dir);
        } else {
            let vendored = manifest.join("../../external/OpenCL-Headers");
            if vendored.join("CL/cl.h").is_file() {
                cfg.define("OpenCL_INCLUDE_DIR", vendored);
            }
        }
    }

    let dst = cfg.build();
    for dir in [dst.join("lib"), dst.join("bin")] {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }

    let link_kind = if shared { "dylib" } else { "static" };
    let libs = if shared {
        vec![
            "sllm_ffi",
            "mtmd",
            "llama-common",
            "llama",
            "ggml",
            "ggml-base",
        ]
    } else {
        vec![
            "sllm_ffi",
            "mtmd",
            "llama-common",
            "llama-common-base",
            "llama",
            "ggml",
            "ggml-base",
            "ggml-cpu",
            "vendor-hash",
        ]
    };
    for lib in libs {
        println!("cargo:rustc-link-lib={link_kind}={lib}");
    }
    emit_system_libs(android);

    generate_bindings(&manifest, &src, android, &target_arch);
}

fn resolve_source(manifest: &Path) -> PathBuf {
    if let Ok(value) = env::var("SLLM_LLAMA_SRC") {
        let p = PathBuf::from(value);
        return if p.is_absolute() { p } else { manifest.join(p) };
    }
    // Keep the path as-is instead of canonicalizing: on Windows canonicalize
    // adds the \\?\ prefix, which some CMake subprojects handle badly.
    manifest.join("../../external/llama-clover-src")
}

fn configure_host(cfg: &mut cmake::Config, _src: &Path, _manifest: &Path) {
    // Host x86_64 uses the default ggml CPU backend. GGML_NATIVE=OFF keeps the
    // binary portable across the developer machines in this repo.
    cfg.define("GGML_CPU_REPACK", "ON");
}

fn configure_android(cfg: &mut cmake::Config, _src: &Path, target_arch: &str, manifest: &Path) {
    let ndk = resolve_ndk(manifest);
    let host_tag = match env::consts::OS {
        "windows" => "windows-x86_64",
        "linux" => "linux-x86_64",
        "macos" => "darwin-x86_64",
        other => panic!("unsupported Android NDK host OS: {other}"),
    };
    let toolchain = ndk.join("build/cmake/android.toolchain.cmake");
    require_file(&toolchain, "android.toolchain.cmake");

    let abi = match target_arch {
        "aarch64" => "arm64-v8a",
        "arm" => "armeabi-v7a",
        "x86_64" => "x86_64",
        "x86" => "x86",
        other => panic!("unsupported Android target arch: {other}"),
    };
    let platform = env::var("SLLM_ANDROID_PLATFORM").unwrap_or_else(|_| "28".into());
    let stl = env::var("SLLM_ANDROID_STL").unwrap_or_else(|_| "c++_shared".into());
    let variant = env::var("SLLM_CPU_VARIANT").unwrap_or_else(|_| "baseline".into());
    let arm_arch = match variant.as_str() {
        "baseline" => "armv8-a",
        "v82" => "armv8.2-a+dotprod+fp16",
        "v86" => "armv8.6-a+dotprod+i8mm+fp16",
        "v90" => "armv9.2-a+dotprod+i8mm+fp16+sve2",
        other => panic!("unknown SLLM_CPU_VARIANT: {other}"),
    };

    cfg.define("CMAKE_TOOLCHAIN_FILE", &toolchain)
        .define("ANDROID_ABI", abi)
        .define("ANDROID_PLATFORM", format!("android-{platform}"))
        .define("ANDROID_STL", &stl)
        .define("GGML_CPU_ARM_ARCH", arm_arch)
        .define(
            "GGML_CPU_REPACK",
            if variant == "baseline" { "OFF" } else { "ON" },
        )
        // The cmake crate injects `--target=aarch64-linux-android` from its
        // host-side cc probe, which overrides the NDK toolchain's versioned
        // `aarch64-none-linux-android28` target and breaks crt object lookup.
        // Defining the flags ourselves keeps the NDK toolchain in charge.
        .define("CMAKE_C_FLAGS", "")
        .define("CMAKE_CXX_FLAGS", "")
        .define("CMAKE_ASM_FLAGS", "");

    println!("cargo:rerun-if-env-changed=SLLM_ANDROID_STL");
    println!("cargo:rerun-if-env-changed=SLLM_ANDROID_PLATFORM");
    println!(
        "cargo:warning=Android build: NDK={} ABI={} platform=android-{} stl={} cpu={} ({}), host tag {}",
        ndk.display(),
        abi,
        platform,
        stl,
        variant,
        arm_arch,
        host_tag
    );
}

fn resolve_ndk(manifest: &Path) -> PathBuf {
    for key in ["ANDROID_NDK_HOME", "ANDROID_NDK_ROOT", "ANDROID_NDK"] {
        if let Ok(value) = env::var(key) {
            let p = PathBuf::from(value);
            if p.is_dir() {
                return p;
            }
        }
    }

    // Common fallbacks: repo-local SDK copy and the standard Windows/macOS/Linux
    // locations. Keep the list short and predictable.
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(sdk) = env::var("ANDROID_HOME") {
        roots.push(PathBuf::from(sdk).join("ndk"));
    }
    if let Ok(sdk) = env::var("ANDROID_SDK_ROOT") {
        roots.push(PathBuf::from(sdk).join("ndk"));
    }
    if let Ok(local) = env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Android/Sdk/ndk"));
    }
    if let Ok(home) = env::var("HOME") {
        roots.push(PathBuf::from(&home).join("Library/Android/sdk/ndk"));
        roots.push(PathBuf::from(&home).join("Android/Sdk/ndk"));
    }
    roots.push(manifest.join("../../.android-sdk/ndk"));

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let preferred = root.join("28.2.13676358");
        if preferred
            .join("build/cmake/android.toolchain.cmake")
            .is_file()
        {
            return preferred;
        }
        if let Ok(entries) = std::fs::read_dir(&root) {
            let mut versions: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join("build/cmake/android.toolchain.cmake").is_file())
                .collect();
            versions.sort();
            if let Some(p) = versions.pop() {
                return p;
            }
        }
    }

    panic!(
        "Android target requested but no NDK found. Set ANDROID_NDK_HOME to the \
         NDK 28.2.13676358 root (build/cmake/android.toolchain.cmake must exist)."
    );
}

fn emit_system_libs(android: bool) {
    if android {
        // The NDK static libs record these dependencies through CMake, but a
        // Rust link step only sees the directives emitted here.
        for lib in ["log", "dl", "m", "atomic"] {
            println!("cargo:rustc-link-lib={lib}");
        }
        // Rust std on Android links libc++_shared.so; keep the shim on the same
        // runtime unless the caller overrides the STL via CMake.
        println!("cargo:rustc-link-lib=dylib=c++_shared");
    } else if env::consts::OS == "windows" {
        for lib in ["advapi32", "userenv", "ws2_32", "bcrypt", "ntdll"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    } else if env::consts::OS == "linux" {
        for lib in ["dl", "m", "pthread"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
}

fn generate_bindings(manifest: &Path, src: &Path, android: bool, target_arch: &str) {
    if let Some(path) = find_libclang() {
        // edition 2024 makes set_var unsafe; this happens before bindgen reads it
        // and the build script is single-threaded.
        unsafe { env::set_var("LIBCLANG_PATH", &path) };
        println!(
            "cargo:warning=bindgen: using libclang at {}",
            path.display()
        );
    }

    let include = src.join("include");
    let ggml_include = src.join("ggml/include");
    let wrapper = manifest.join("include/sllm_bindgen.h");

    let mut builder = bindgen::Builder::default()
        .header(wrapper.to_string_lossy())
        .clang_arg("-x")
        .clang_arg("c++")
        .clang_arg("-std=c++17")
        .clang_arg(format!("-I{}", include.display()))
        .clang_arg(format!("-I{}", ggml_include.display()))
        .allowlist_function("llama_.*")
        .allowlist_function("ggml_.*")
        .allowlist_function("sllm_.*")
        .allowlist_type("llama_.*")
        .allowlist_type("ggml_.*")
        .allowlist_type("sllm_.*")
        .allowlist_var("LLAMA_.*")
        .allowlist_var("GGML_.*")
        .allowlist_var("SLLM_.*")
        .rustified_enum("llama_.*")
        .rustified_enum("ggml_.*")
        .generate_comments(true)
        .layout_tests(true)
        .formatter(bindgen::Formatter::Rustfmt);

    if android {
        let ndk = resolve_ndk(manifest);
        let host_tag = match env::consts::OS {
            "windows" => "windows-x86_64",
            "linux" => "linux-x86_64",
            "macos" => "darwin-x86_64",
            other => panic!("unsupported Android NDK host OS: {other}"),
        };
        let sysroot = ndk.join(format!("toolchains/llvm/prebuilt/{host_tag}/sysroot"));
        let clang_root = ndk.join(format!("toolchains/llvm/prebuilt/{host_tag}/lib/clang"));
        let resource_dir = newest_dir(&clang_root);
        let target = match target_arch {
            "aarch64" => "aarch64-linux-android28",
            "arm" => "armv7a-linux-androideabi28",
            "x86_64" => "x86_64-linux-android28",
            "x86" => "i686-linux-android28",
            other => panic!("unsupported Android target arch: {other}"),
        };
        builder = builder
            .clang_arg(format!("--target={target}"))
            .clang_arg(format!("--sysroot={}", sysroot.display()));
        if let Some(resource_dir) = resource_dir {
            // The host libclang wheel has no builtin headers of its own; borrow
            // the NDK's clang resource directory (stdarg.h, stddef.h, ...).
            builder = builder.clang_arg(format!("-resource-dir={}", resource_dir.display()));
        }
    }

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    builder
        .generate()
        .unwrap_or_else(|e| panic!("bindgen failed: {e}"))
        .write_to_file(out.join("bindings.rs"))
        .expect("write bindings.rs");
}

fn find_libclang() -> Option<PathBuf> {
    if let Ok(path) = env::var("LIBCLANG_PATH") {
        let p = PathBuf::from(path);
        if p.is_dir() {
            return Some(p);
        }
    }

    if let Ok(path) = env::var("SLLM_LIBCLANG_PATH") {
        let p = PathBuf::from(path);
        if p.is_dir() {
            return Some(p);
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    // Android NDK ships clang but not libclang; still worth checking.
    if let Ok(ndk) = env::var("ANDROID_NDK_HOME") {
        let host_tag = if env::consts::OS == "windows" {
            "windows-x86_64"
        } else if env::consts::OS == "linux" {
            "linux-x86_64"
        } else {
            "darwin-x86_64"
        };
        candidates
            .push(PathBuf::from(ndk).join(format!("toolchains/llvm/prebuilt/{host_tag}/bin")));
    }

    // Python's libclang wheel is the most common Windows install in this repo's
    // dev environments and is detected automatically.
    if env::consts::OS == "windows" {
        for base in ["C:\\Python312", "C:\\Python311", "C:\\Python310"] {
            candidates.push(PathBuf::from(base).join("Lib/site-packages/clang/native"));
        }
        if let Ok(local) = env::var("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(&local)
                    .join("Programs/Python/Python312/Lib/site-packages/clang/native"),
            );
            candidates.push(
                PathBuf::from(&local)
                    .join("Programs/Python/Python311/Lib/site-packages/clang/native"),
            );
        }
    }
    if env::consts::OS == "macos" {
        candidates.push(PathBuf::from("/opt/homebrew/opt/llvm/lib"));
        candidates.push(PathBuf::from("/usr/local/opt/llvm/lib"));
    }
    if env::consts::OS == "linux" {
        candidates.push(PathBuf::from("/usr/lib/llvm-18/lib"));
        candidates.push(PathBuf::from("/usr/lib/llvm-17/lib"));
        candidates.push(PathBuf::from("/usr/lib/llvm-16/lib"));
        candidates.push(PathBuf::from("/usr/lib/x86_64-linux-gnu"));
    }
    candidates.push(PathBuf::from("/usr/lib/llvm/lib"));

    for dir in candidates {
        if dir.is_dir() {
            let has_lib = std::fs::read_dir(&dir)
                .ok()
                .map(|entries| {
                    entries.flatten().any(|e| {
                        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                        name.starts_with("libclang.")
                            || name == "clang.dll"
                            || name == "libclang.dll"
                    })
                })
                .unwrap_or(false);
            if has_lib {
                return Some(dir);
            }
        }
    }

    // macOS/Linux: ask the loader.
    if env::consts::OS != "windows" {
        for name in [
            "libclang.dylib",
            "libclang.so",
            "libclang-18.so",
            "libclang-17.so",
        ] {
            if let Ok(output) = Command::new("sh")
                .arg("-c")
                .arg(format!("command -v ldconfig >/dev/null 2>&1 && ldconfig -p | grep -m1 {name} | awk '{{print $NF}}'"))
                .output()
            {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !text.is_empty()
                    && let Some(parent) = Path::new(&text).parent()
                {
                    return Some(parent.to_path_buf());
                }
            }
        }
    }

    println!(
        "cargo:warning=libclang not found; set LIBCLANG_PATH to the directory \
         containing libclang.dll/.so/.dylib. bindgen will fail without it."
    );
    None
}

fn newest_dir(root: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs.pop()
}

fn env_flag(key: &str) -> bool {
    match env::var(key) {
        Ok(value) => {
            let value = value.trim();
            !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
        }
        Err(_) => false,
    }
}

fn require_file(path: &Path, what: &str) {
    if !path.is_file() {
        panic!(
            "sllm-llama-sys: {what} not found at {}. Set SLLM_LLAMA_SRC to the \
             pinned llama.cpp source tree (default external/llama-clover-src).",
            path.display()
        );
    }
}
