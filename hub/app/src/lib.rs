//! ABM Local AI hub shell.
//!
//! The window is a model library: it shows what this computer has, which models run comfortably
//! on it (the rest are greyed out with the minimum specs), downloads and verifies models, and
//! lets the user pick the one that runs. Pairing, per-app keys and the policy API arrive in M1.
//!
//! Developer overrides: `ABM_MODEL` (a .gguf path to load instead of the active pack) and
//! `ABM_LLAMA_DIR` (a llama.cpp folder instead of the bundled `llama/` resource).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use abm_hub_core::catalog::{Catalog, Compatibility, Model};
use abm_hub_core::hardware::Hardware;
use abm_hub_core::packs::{Download, Packs, Settings};
use abm_hub_core::{http, platform, LlamaConfig, State, Supervisor};
use serde::Serialize;
use serde_json::json;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, RunEvent};

struct Hub {
    supervisor: Supervisor,
    llama_dir: PathBuf,
    data_dir: PathBuf,
    catalog: Arc<Catalog>,
    packs: Packs,
    hardware: Mutex<Hardware>,
    settings: Mutex<Settings>,
    /// What the supervisor was last asked to run: a catalog id, or a developer override path.
    running: Mutex<Option<Running>>,
}

#[derive(Clone, Serialize)]
struct Running {
    id: Option<String>,
    name: String,
    kind: String,
}

#[derive(Serialize)]
struct ModelView {
    #[serde(flatten)]
    model: Model,
    download_gb: f64,
    compatibility: Compatibility,
    installed: bool,
    download: Option<Download>,
    active: bool,
    recommended: bool,
}

#[derive(Serialize)]
struct Library {
    version: &'static str,
    platform: &'static str,
    llama_build: Option<String>,
    packs_dir: String,
    hardware: Hardware,
    models: Vec<ModelView>,
    server: State,
    running: Option<Running>,
}

#[derive(Serialize)]
struct PromptResult {
    text: String,
    total_ms: u128,
    tokens: u64,
    tokens_per_s: Option<f64>,
}

fn llama_build(dir: &Path) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("llama.json")).ok()?).ok()?;
    v["build"].as_str().map(str::to_owned)
}

fn config_for(hub: &Hub, m: &Model, path: PathBuf) -> LlamaConfig {
    let mut cfg = LlamaConfig::new(&hub.llama_dir, path);
    cfg.ctx_size = m.context;
    cfg.extra_args = m.server_args.clone();
    cfg
}

/// Load a pack: verify it, start llama-server and remember it as the active model.
fn activate(hub: &Hub, id: &str) -> Result<(), String> {
    let m = hub.catalog.get(id).ok_or("Unknown model")?.clone();
    let path = hub.packs.verify_for_load(&m).map_err(|e| e.to_string())?;
    hub.supervisor.start(config_for(hub, &m, path));
    *hub.running.lock().unwrap() = Some(Running { id: Some(m.id.clone()), name: m.name.clone(), kind: m.kind.clone() });
    let mut s = hub.settings.lock().unwrap();
    s.active = Some(m.id.clone());
    s.save(&hub.data_dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn library(hub: tauri::State<'_, Hub>) -> Library {
    let hardware = {
        let mut hw = hub.hardware.lock().unwrap();
        hw.refresh_disk(&hub.data_dir);
        hw.clone()
    };
    let active = hub.settings.lock().unwrap().active.clone();
    let mut models: Vec<ModelView> = hub
        .catalog
        .models
        .iter()
        .map(|m| {
            let installed = hub.packs.is_installed(m);
            ModelView {
                download_gb: m.download_gb(),
                compatibility: m.compatibility(&hardware, installed),
                installed,
                download: hub.packs.download_status(&m.id),
                active: active.as_deref() == Some(m.id.as_str()),
                recommended: false,
                model: m.clone(),
            }
        })
        .collect();
    // Recommend the most capable chat model that runs comfortably (the catalog is ordered by size).
    if let Some(best) = models.iter_mut().rev().find(|v| v.model.kind == "chat" && v.compatibility.ok) {
        best.recommended = true;
    }
    Library {
        version: env!("CARGO_PKG_VERSION"),
        platform: platform::NAME,
        llama_build: llama_build(&hub.llama_dir),
        packs_dir: hub.data_dir.join("packs").display().to_string(),
        hardware,
        models,
        server: hub.supervisor.state(),
        running: hub.running.lock().unwrap().clone(),
    }
}

#[tauri::command]
fn download_model(hub: tauri::State<'_, Hub>, id: String) -> Result<(), String> {
    let hw = hub.hardware.lock().unwrap().clone();
    let m = hub.catalog.get(&id).ok_or("Unknown model")?;
    if !m.compatibility(&hw, false).ok {
        return Err(format!("{} can't run comfortably on this computer.", m.name));
    }
    hub.packs.start_download(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn pause_download(hub: tauri::State<'_, Hub>, id: String) {
    hub.packs.pause_download(&id);
}

#[tauri::command]
fn remove_model(hub: tauri::State<'_, Hub>, id: String) -> Result<(), String> {
    let running_this = hub.running.lock().unwrap().as_ref().and_then(|r| r.id.clone()).as_deref() == Some(id.as_str());
    if running_this {
        hub.supervisor.stop();
        hub.supervisor.wait_settled(Duration::from_secs(15));
        *hub.running.lock().unwrap() = None;
    }
    {
        let mut s = hub.settings.lock().unwrap();
        if s.active.as_deref() == Some(id.as_str()) {
            s.active = None;
            s.save(&hub.data_dir).map_err(|e| e.to_string())?;
        }
    }
    hub.packs.remove(&id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn use_model(app: tauri::AppHandle, id: String) -> Result<(), String> {
    // Verification can re-hash a large file, so keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(move || activate(&app.state::<Hub>(), &id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn stop_model(hub: tauri::State<'_, Hub>) {
    hub.supervisor.stop();
    *hub.running.lock().unwrap() = None;
    let mut s = hub.settings.lock().unwrap();
    s.active = None;
    let _ = s.save(&hub.data_dir);
}

#[tauri::command]
async fn test_prompt(app: tauri::AppHandle, prompt: String) -> Result<PromptResult, String> {
    let ep = app.state::<Hub>().supervisor.endpoint().ok_or("The model is not running.")?;
    tauri::async_runtime::spawn_blocking(move || {
        let body = json!({
            "messages": [{ "role": "user", "content": prompt }],
            "max_tokens": 200,
            "temperature": 0
        });
        let bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        let t0 = Instant::now();
        let r = http::request(ep.port, "POST", "/v1/chat/completions", Some(&ep.api_key), Some(&bytes), Duration::from_secs(300))
            .map_err(|e| e.to_string())?;
        let total_ms = t0.elapsed().as_millis();
        if r.status != 200 {
            return Err(format!("The model returned an error ({}).", r.status));
        }
        let v = r.json().map_err(|e| e.to_string())?;
        Ok(PromptResult {
            text: v["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_owned(),
            total_ms,
            tokens: v["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            tokens_per_s: v["timings"]["predicted_per_second"].as_f64(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let llama_dir = std::env::var_os("ABM_LLAMA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| app.path().resource_dir().unwrap_or_default().join("llama"));
            let data_dir = platform::data_dir();
            std::fs::create_dir_all(&data_dir)?;
            let catalog = Arc::new(Catalog::builtin());
            let packs = Packs::new(&data_dir, Arc::clone(&catalog));
            let hub = Hub {
                supervisor: Supervisor::new(),
                hardware: Mutex::new(Hardware::detect(&data_dir)),
                settings: Mutex::new(Settings::load(&data_dir)),
                running: Mutex::new(None),
                llama_dir,
                data_dir,
                catalog,
                packs,
            };
            if let Some(p) = std::env::var_os("ABM_MODEL").map(PathBuf::from) {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                hub.supervisor.start(LlamaConfig::new(&hub.llama_dir, &p));
                *hub.running.lock().unwrap() = Some(Running { id: None, name, kind: "chat".into() });
            } else {
                // Take the saved id in its own statement: in an `if let` the lock guard would
                // live through the block and deadlock when activate() locks settings again.
                let saved = hub.settings.lock().unwrap().active.clone();
                if let Some(id) = saved {
                    // Restart the model the user picked last time. A failure shows in the window.
                    let _ = activate_later(&hub, &id);
                }
            }
            app.manage(hub);

            // The window works fully without the tray: stock GNOME hides tray icons unless the
            // AppIndicator extension is installed.
            let show = MenuItem::with_id(app, "show", "Open ABM Local AI", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::with_id("hub").tooltip("ABM Local AI").menu(&menu).on_menu_event(
                |app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                },
            );
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            library,
            download_model,
            pause_download,
            remove_model,
            use_model,
            stop_model,
            test_prompt
        ])
        .build(tauri::generate_context!())
        .expect("error while building ABM Local AI");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            // Stop llama-server before the process exits; the platform guard covers crashes.
            let hub = app.state::<Hub>();
            hub.supervisor.stop();
            hub.supervisor.wait_settled(Duration::from_secs(15));
        }
    });
}

/// At startup, start the saved active pack if it is still installed. Verification is a quick
/// size and date check unless the file changed since install.
fn activate_later(hub: &Hub, id: &str) -> Result<(), String> {
    match hub.catalog.get(id) {
        Some(m) if hub.packs.is_installed(m) => activate(hub, id),
        _ => Err("saved model is no longer installed".into()),
    }
}
