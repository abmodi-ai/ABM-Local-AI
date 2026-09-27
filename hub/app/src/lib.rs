//! ABM Local AI hub shell (M0 skeleton).
//!
//! Proves that the Tauri app can bundle llama-server as a resource folder and supervise it on
//! Windows, macOS and Linux. It adds a tray icon, a status window, start/stop and a test prompt.
//! Pairing, per-app keys and the policy API arrive in M1.
//!
//! The model comes from `ABM_MODEL` or from the first `.gguf` in `<data dir>/packs/`. The
//! llama.cpp folder comes from `ABM_LLAMA_DIR` or the bundled `llama/` resource.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
    model: std::sync::Mutex<Option<PathBuf>>,
}

#[derive(Serialize)]
struct Status {
    version: &'static str,
    platform: &'static str,
    arch: &'static str,
    llama_build: Option<String>,
    llama_dir: String,
    data_dir: String,
    model: Option<String>,
    server: State,
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

fn find_model(data_dir: &Path) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ABM_MODEL") {
        return Some(PathBuf::from(p));
    }
    let mut ggufs: Vec<PathBuf> = std::fs::read_dir(data_dir.join("packs"))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("gguf")))
        .collect();
    ggufs.sort();
    ggufs.into_iter().next()
}

#[tauri::command]
fn hub_status(hub: tauri::State<'_, Hub>) -> Status {
    Status {
        version: env!("CARGO_PKG_VERSION"),
        platform: platform::NAME,
        arch: std::env::consts::ARCH,
        llama_build: llama_build(&hub.llama_dir),
        llama_dir: hub.llama_dir.display().to_string(),
        data_dir: hub.data_dir.display().to_string(),
        model: hub.model.lock().unwrap().as_ref().map(|p| p.display().to_string()),
        server: hub.supervisor.state(),
    }
}

#[tauri::command]
fn start_model(hub: tauri::State<'_, Hub>, path: Option<String>) -> Result<(), String> {
    let mut model = hub.model.lock().unwrap();
    if let Some(p) = path.filter(|p| !p.trim().is_empty()) {
        *model = Some(PathBuf::from(p.trim()));
    }
    let m = model.clone().ok_or("No model selected. Enter a .gguf path or put one in the packs folder.")?;
    hub.supervisor.start(LlamaConfig::new(&hub.llama_dir, m));
    Ok(())
}

#[tauri::command]
fn stop_model(hub: tauri::State<'_, Hub>) {
    hub.supervisor.stop();
}

#[tauri::command]
async fn test_prompt(app: tauri::AppHandle, prompt: String) -> Result<PromptResult, String> {
    let ep = app.state::<Hub>().supervisor.endpoint().ok_or("The model is not running.")?;
    tauri::async_runtime::spawn_blocking(move || {
        let body = json!({
            "messages": [{ "role": "user", "content": prompt }],
            "max_tokens": 200,
            "temperature": 0,
            "chat_template_kwargs": { "enable_thinking": false }
        });
        let bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        let t0 = Instant::now();
        let r = http::request(ep.port, "POST", "/v1/chat/completions", Some(&ep.api_key), Some(&bytes), Duration::from_secs(300))
            .map_err(|e| e.to_string())?;
        let total_ms = t0.elapsed().as_millis();
        if r.status != 200 {
            return Err(format!("llama-server returned {}", r.status));
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
            std::fs::create_dir_all(data_dir.join("packs"))?;
            let model = find_model(&data_dir);
            let supervisor = Supervisor::new();
            if let Some(m) = &model {
                supervisor.start(LlamaConfig::new(&llama_dir, m));
            }
            app.manage(Hub { supervisor, llama_dir, data_dir, model: std::sync::Mutex::new(model) });

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
        .invoke_handler(tauri::generate_handler![hub_status, start_model, stop_model, test_prompt])
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
