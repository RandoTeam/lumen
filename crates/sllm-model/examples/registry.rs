//! Print the discovered registry as JSON.
//!
//! ```text
//! cargo run -p sllm-model --example registry -- models 8192
//! ```

use std::path::PathBuf;

use sllm_model::ModelRegistry;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("models"));
    let avail_mb = args
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(8192);

    let registry = match ModelRegistry::discover(&root) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("registry discovery failed: {error}");
            std::process::exit(1);
        }
    };

    match registry.to_json_pretty(avail_mb) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("registry serialization failed: {error}");
            std::process::exit(1);
        }
    }
}
