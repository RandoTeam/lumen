//! Linker flags for the Android cdylib.
//!
//! Android 15+ devices can use 16 KB pages. The final `libsnapdragonllmweb.so`
//! must be aligned for them even if every static library is linked into it.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "android" {
        println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
    }
}
