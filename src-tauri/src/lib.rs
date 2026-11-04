mod card;
mod sidecar;

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::ipc::CapabilityBuilder;
use tauri::menu::{Menu, MenuItem, Submenu};
use serde_json::Value;
use tauri::{AppHandle, Manager, RunEvent, State, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::oneshot;

use card::{CardInfo, Cards, PinRequest};
use sidecar::Sidecar;

const MAIN: &str = "main";
const SETUP: &str = "setup";
const PIN: &str = "pin";
const SESSION_MINUTES: u64 = 240;

/// Persisted in `<app config dir>/config.json`.
#[derive(Default, Serialize, Deserialize)]
struct Config {
    url: Option<String>,
    /// Path of the PKCS#11 library; default: the bit4id locations in `card.rs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pkcs11_library: Option<String>,
    /// Lifetime of a card session after the PIN; default 240.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_minutes: Option<u64>,
}

/// A `card_sign` waiting for the PIN window.
struct PendingPin {
    fingerprint: String,
    reply: oneshot::Sender<Result<(), String>>,
}

#[derive(Default)]
struct Signer {
    /// Loaded on the first card command, so the runner works without the library.
    cards: Mutex<Option<Cards>>,
    pending: Mutex<Option<PendingPin>>,
    /// Origins that already received the card capability.
    origins: Mutex<HashSet<String>>,
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

fn load_config(app: &AppHandle) -> Result<Config, String> {
    let path = config_path(app)?;
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn save_config(app: &AppHandle, config: &Config) -> Result<(), String> {
    let path = config_path(app)?;
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn parse_url(text: &str) -> Result<Url, String> {
    let url = Url::parse(text.trim()).map_err(|e| format!("Invalid URL: {e}"))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(format!("Unsupported scheme: {other}")),
    }
}

/// Value of `--url <address>` on the command line.
fn cli_url() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--url" {
            return args.next();
        }
    }
    None
}

/// Grants the card commands to pages of the origin of `url` in the main window.
/// A grant stays until the app exits, also after Change URL.
fn grant_card_commands(app: &AppHandle, url: &Url) -> tauri::Result<()> {
    let origin = url.origin().ascii_serialization();
    let signer = app.state::<Signer>();
    let mut origins = signer.origins.lock().unwrap();
    if origins.contains(&origin) {
        return Ok(());
    }
    let capability = CapabilityBuilder::new(format!("card-{}", origins.len()))
        .remote(format!("{origin}/*"))
        .local(false)
        .window(MAIN)
        .permission("allow-card-list")
        .permission("allow-card-sign")
        .permission("allow-card-close")
        .permission("allow-device-call")
        .permission("core:event:default");
    app.add_capability(capability)?;
    origins.insert(origin);
    Ok(())
}

/// Loads `url` in the main window, creating the window when it does not exist.
fn show_instance(app: &AppHandle, url: Url) -> tauri::Result<()> {
    grant_card_commands(app, &url)?;
    let title = url.host_str().unwrap_or("GenroPy").to_string();
    match app.get_webview_window(MAIN) {
        Some(window) => {
            window.navigate(url)?;
            window.set_title(&title)?;
            window.set_focus()?;
        }
        None => {
            WebviewWindowBuilder::new(app, MAIN, WebviewUrl::External(url))
                .title(title)
                .inner_size(1280.0, 800.0)
                .build()?;
        }
    }
    Ok(())
}

fn open_setup(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(SETUP) {
        return window.set_focus();
    }
    WebviewWindowBuilder::new(app, SETUP, WebviewUrl::App("index.html".into()))
        .title("Instance URL")
        .inner_size(520.0, 240.0)
        .resizable(false)
        .build()?;
    Ok(())
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::default(app)?;
    let change_url = MenuItem::with_id(app, "change_url", "Change URL…", true, Some("CmdOrCtrl+Shift+U"))?;
    let reload = MenuItem::with_id(app, "reload", "Reload", true, Some("CmdOrCtrl+R"))?;
    let instance = Submenu::with_items(app, "Instance", true, &[&change_url, &reload])?;
    menu.append(&instance)?;
    Ok(menu)
}

fn on_menu(app: &AppHandle, id: &str) -> tauri::Result<()> {
    match id {
        "change_url" => open_setup(app),
        "reload" => match app.get_webview_window(MAIN) {
            Some(window) => window.eval("window.location.reload()"),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

/// Runs `action` on the cards, loading the library and re-reading the readers first.
fn with_cards<T>(app: &AppHandle, action: impl FnOnce(&mut Cards) -> Result<T, String>) -> Result<T, String> {
    let signer = app.state::<Signer>();
    let mut guard = signer.cards.lock().unwrap();
    if guard.is_none() {
        let library = card::library_path(load_config(app)?.pkcs11_library.as_deref())?;
        *guard = Some(Cards::load(&library)?);
    }
    let cards = guard.as_mut().unwrap();
    cards.refresh()?;
    action(cards)
}

/// Opens the PIN window for `fingerprint` and waits until the card is open or the user gives up.
async fn ask_pin(app: &AppHandle, fingerprint: &str) -> Result<(), String> {
    let (reply, answer) = oneshot::channel();
    {
        let signer = app.state::<Signer>();
        let mut pending = signer.pending.lock().unwrap();
        if pending.is_some() {
            return Err("another PIN request is open".to_string());
        }
        *pending = Some(PendingPin { fingerprint: fingerprint.to_string(), reply });
    }
    let opened = WebviewWindowBuilder::new(app, PIN, WebviewUrl::App("pin.html".into()))
        .title("Card PIN")
        .inner_size(420.0, 230.0)
        .resizable(false)
        .always_on_top(true)
        .build();
    if let Err(e) = opened {
        app.state::<Signer>().pending.lock().unwrap().take();
        return Err(e.to_string());
    }
    answer.await.unwrap_or_else(|_| Err("PIN cancelled".to_string()))
}

/// Ends the pending PIN request with `result` and closes the PIN window.
fn finish_pin(app: &AppHandle, result: Result<(), String>) -> Result<(), String> {
    if let Some(pending) = app.state::<Signer>().pending.lock().unwrap().take() {
        let _ = pending.reply.send(result);
    }
    if let Some(window) = app.get_webview_window(PIN) {
        window.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn pending_fingerprint(signer: &Signer) -> Result<String, String> {
    signer
        .pending
        .lock()
        .unwrap()
        .as_ref()
        .map(|pending| pending.fingerprint.clone())
        .ok_or_else(|| "no PIN request is open".to_string())
}

// ------------------------------------------------------------------ setup window

#[tauri::command]
async fn get_url(app: AppHandle) -> Result<Option<String>, String> {
    Ok(load_config(&app)?.url)
}

#[tauri::command]
async fn save_url(app: AppHandle, url: String) -> Result<(), String> {
    let target = parse_url(&url)?;
    let mut config = load_config(&app)?;
    config.url = Some(target.to_string());
    save_config(&app, &config)?;
    show_instance(&app, target).map_err(|e| e.to_string())?;
    if let Some(window) = app.get_webview_window(SETUP) {
        window.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ------------------------------------------------------------------ PIN window

#[tauri::command]
async fn pin_request(app: AppHandle, signer: State<'_, Signer>) -> Result<PinRequest, String> {
    let fingerprint = pending_fingerprint(&signer)?;
    with_cards(&app, |cards| cards.pin_request(&fingerprint))
}

/// One login attempt. On error the window stays open and shows the message.
#[tauri::command]
async fn pin_submit(app: AppHandle, signer: State<'_, Signer>, pin: String) -> Result<(), String> {
    let fingerprint = pending_fingerprint(&signer)?;
    let minutes = load_config(&app)?.session_minutes.unwrap_or(SESSION_MINUTES);
    let until = SystemTime::now() + Duration::from_secs(minutes * 60);
    with_cards(&app, |cards| cards.open(&fingerprint, &pin, until))?;
    finish_pin(&app, Ok(()))
}

#[tauri::command]
async fn pin_cancel(app: AppHandle) -> Result<(), String> {
    finish_pin(&app, Err("PIN cancelled".to_string()))
}

// ------------------------------------------------------------------ GenroPy page

#[tauri::command]
async fn card_list(app: AppHandle) -> Result<Vec<CardInfo>, String> {
    with_cards(&app, |cards| Ok(cards.list()))
}

/// Signature (base64) of `digest` (base64 of 32 bytes) with the card `card`.
/// A closed card first asks its PIN in a local window: the page never sees the PIN.
#[tauri::command]
async fn card_sign(app: AppHandle, card: String, digest: String) -> Result<String, String> {
    let digest = STANDARD.decode(digest.trim()).map_err(|e| format!("digest is not base64: {e}"))?;
    if !with_cards(&app, |cards| Ok(cards.is_open(&card)))? {
        ask_pin(&app, &card).await?;
    }
    let signature = with_cards(&app, |cards| cards.sign(&card, &digest))?;
    Ok(STANDARD.encode(signature))
}

#[tauri::command]
async fn card_close(app: AppHandle, card: String) -> Result<(), String> {
    with_cards(&app, |cards| cards.close(&card))
}

// ------------------------------------------------------------------ Python sidecar

/// Calls `module.method(**params)` in the Python sidecar.
#[tauri::command]
async fn device_call(
    sidecar: State<'_, Sidecar>,
    module: String,
    method: String,
    params: Option<Value>,
) -> Result<Value, String> {
    sidecar.call(&module, &method, params.unwrap_or(Value::Null)).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&[SETUP, PIN])
                .build(),
        )
        .manage(Signer::default())
        .invoke_handler(tauri::generate_handler![
            get_url,
            save_url,
            pin_request,
            pin_submit,
            pin_cancel,
            card_list,
            card_sign,
            card_close,
            device_call
        ])
        .on_window_event(|window, event| {
            // Closing the PIN window with its close button cancels the request.
            if window.label() == PIN && matches!(event, WindowEvent::Destroyed) {
                if let Some(pending) = window.state::<Signer>().pending.lock().unwrap().take() {
                    let _ = pending.reply.send(Err("PIN cancelled".to_string()));
                }
            }
        })
        .setup(|app| {
            let handle = app.handle();
            let sidecar = Sidecar::start(handle.clone(), MAIN, &config_path(handle)?)?;
            handle.manage(sidecar);
            handle.set_menu(build_menu(handle)?)?;
            handle.on_menu_event(|app, event| {
                if let Err(e) = on_menu(app, event.id().as_ref()) {
                    eprintln!("menu {}: {e}", event.id().as_ref());
                }
            });
            // Precedence: --url, then config.json, then the setup page.
            let url = match cli_url() {
                Some(text) => Some(text),
                None => load_config(handle)?.url,
            };
            match url {
                Some(text) => show_instance(handle, parse_url(&text)?)?,
                None => open_setup(handle)?,
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<Sidecar>().stop();
            }
        });
}
