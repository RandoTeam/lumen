use std::path::PathBuf;

use sllm_core::config::EngineConfig;
use sllm_core::gguf;
use sllm_core::model;

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "scan".to_string());
    match command.as_str() {
        "scan" => {
            let root = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("models"));
            match model::scan_dir(&root) {
                Ok(models) => {
                    println!(
                        "{:<32} {:>10} {:<12} {:<10} {:>8}",
                        "name", "GB", "arch", "quant", "est MB"
                    );
                    for m in models {
                        let pf = gguf::preflight(&m.path, m.bytes, 8000, &EngineConfig::default());
                        println!(
                            "{:<32} {:>10.2} {:<12} {:<10} {:>8}",
                            m.name,
                            m.size_gb(),
                            m.meta.architecture.as_deref().unwrap_or("?"),
                            m.quant(),
                            pf.estimate_mb
                        );
                        for w in pf.warnings {
                            println!("    ! {w}");
                        }
                    }
                }
                Err(e) => eprintln!("scan failed: {e}"),
            }
        }
        "check" => {
            let Some(path) = args.next().map(PathBuf::from) else {
                eprintln!("usage: sllm-cli check <model.gguf>");
                return;
            };
            let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let pf = gguf::preflight(&path, bytes, 8000, &EngineConfig::default());
            println!(
                "ok={} code={:?} arch={:?} quant={:?} est={} MB",
                pf.ok, pf.code, pf.arch, pf.quant, pf.estimate_mb
            );
            if let Some(msg) = pf.message {
                println!("message: {msg}");
            }
            if let Some(detail) = pf.detail {
                println!("detail: {detail}");
            }
            for w in pf.warnings {
                println!("warning: {w}");
            }
        }
        _ => {
            eprintln!("usage: sllm-cli [scan [dir] | check <model.gguf>]");
        }
    }
}
