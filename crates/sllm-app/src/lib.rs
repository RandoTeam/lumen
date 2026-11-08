slint::include_modules!();

use slint::Model;

fn demo_messages() -> Vec<Message> {
    vec![
        Message {
            role: "YOU".into(),
            text: "Explain how mixture-of-experts models work, in five short bullet points.".into(),
            meta: "".into(),
        },
        Message {
            role: "CLOVER 35B".into(),
            text: "1. A router picks a small set of experts for each token.\n2. Only those experts run, so active compute stays small.\n3. Experts specialize during training, not by hand.\n4. The full model is large, but the active path is cheap.\n5. On a phone the bottleneck is reading experts from storage.".into(),
            meta: "18.4 tok/s · 5.1 s TTFT · 38% context".into(),
        },
    ]
}

fn model_item(m: &sllm_core::ModelEntry) -> ModelItem {
    let pf = sllm_core::gguf::preflight(
        &m.path,
        m.bytes,
        8000,
        &sllm_core::config::EngineConfig::default(),
    );
    let status = if pf.ok {
        "Ready".to_string()
    } else {
        pf.message
            .unwrap_or_else(|| "Unsupported format".to_string())
    };
    let arch = m
        .meta
        .architecture
        .clone()
        .unwrap_or_else(|| "GGUF".to_string());
    let layers = m.meta.block_count.unwrap_or(0);
    let meta = if m.is_moe() {
        format!("{arch} · {} · {layers} layers", m.kind())
    } else {
        let ctx = m.meta.context_length.unwrap_or(0);
        format!("{arch} · {layers} layers · {ctx} ctx")
    };
    ModelItem {
        name: m.name.clone().into(),
        meta: meta.into(),
        size: format!("{:.2} GB", m.size_gb()).into(),
        quant: m.quant().into(),
        status: status.into(),
        active: false,
    }
}

fn scan_models(dir: &std::path::Path) -> Vec<ModelItem> {
    match sllm_core::model::scan_dir(dir) {
        Ok(entries) => {
            let mut seen = std::collections::HashSet::new();
            entries
                .iter()
                .filter(|m| seen.insert((m.name.clone(), m.bytes)))
                .map(model_item)
                .collect()
        }
        Err(e) => {
            log::warn!("scan {} failed: {e}", dir.display());
            Vec::new()
        }
    }
}

fn build_ui(models: Vec<ModelItem>) -> AppWindow {
    let ui = AppWindow::new().expect("create AppWindow");
    ui.set_models(slint::ModelRc::new(slint::VecModel::from(models)));

    let messages = std::rc::Rc::new(slint::VecModel::from(demo_messages()));
    ui.set_messages(slint::ModelRc::from(messages.clone()));
    ui.on_submitted(move |text| {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        log::info!("submitted: {text}");
        messages.push(Message {
            role: "YOU".into(),
            text: text.into(),
            meta: "".into(),
        });
        messages.push(Message {
            role: "SLLM".into(),
            text: "Engine is not connected yet. The Rust UI, GGUF preflight and model scan work; inference lands with sllm-llama-sys.".into(),
            meta: "local placeholder".into(),
        });
        log::info!("messages now: {}", messages.row_count());
    });

    ui
}

#[cfg(not(target_os = "android"))]
pub fn run_desktop() {
    let ui = build_ui(scan_models(std::path::Path::new("models")));
    ui.run().expect("run desktop UI");
}

#[cfg(target_os = "android")]
mod android {
    use super::*;
    use slint::android::android_activity::{MainEvent, PollEvent};

    #[unsafe(no_mangle)]
    pub extern "C" fn android_main(app: slint::android::AndroidApp) {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("SLLM"),
        );
        log::info!("android_main: start");

        let models_dir = app
            .external_data_path()
            .or_else(|| app.internal_data_path())
            .map(|p| p.join("models"));

        let app_for_events = app.clone();
        slint::android::init_with_event_listener(app, move |event| {
            if let PollEvent::Main(me) = event {
                log::info!("lifecycle: {me:?}");
                if let MainEvent::Destroy = me {
                    log::info!("activity destroy");
                }
            }
        })
        .expect("slint android backend init");

        // Show the shell immediately; scan GGUF metadata off the UI thread.
        let ui = build_ui(Vec::new());
        if let Some(dir) = models_dir {
            log::info!("scanning models in {}", dir.display());
            let weak = ui.as_weak();
            std::thread::spawn(move || {
                let models = scan_models(&dir);
                log::info!("models found: {}", models.len());
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        log::info!("applying {} models to UI", models.len());
                        ui.set_models(slint::ModelRc::new(slint::VecModel::from(models)));
                    }
                });
            });
        }

        log::info!("slint event loop start");
        ui.run().expect("slint event loop");
        log::info!("slint event loop returned");
        let _ = app_for_events;
    }
}
