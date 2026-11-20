mod card;
mod connector;
mod documents;
use documents::{Document, Registry};
#[cfg(target_os = "macos")]
mod picker;
mod sidecar;

use std::collections::HashSet;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::ipc::CapabilityBuilder;
use tauri::menu::{Menu, MenuItem, Submenu};
use serde_json::Value;
use tauri::{Emitter, AppHandle, Manager, RunEvent, State, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::oneshot;

use card::{CardInfo, Cards, PinRequest};
use sidecar::Sidecar;

const SUPPORT: &str = "support";
const MEETLOT: &str = "meetlot";
const SETUP: &str = "setup";
const PIN: &str = "pin";
const SESSION_MINUTES: u64 = 240;

/// Persisted in `<app config dir>/config.json`.
#[derive(Default, Serialize, Deserialize)]
struct Config {
    url: Option<String>,
    document: Option<Document>,
    #[serde(flatten)]
    other: std::collections::BTreeMap<String, Value>,
    /// Paths of PKCS#11 libraries, tried before the default locations in `card.rs`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pkcs11_libraries: Vec<String>,
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
fn grant_card_commands(app: &AppHandle, url: &Url, label: &str) -> tauri::Result<()> {
    let origin = url.origin().ascii_serialization();
    let signer = app.state::<Signer>();
    let mut origins = signer.origins.lock().unwrap();
    if origins.contains(label) {
        return Ok(());
    }
    let capability = CapabilityBuilder::new(format!("card-{}", origins.len()))
        .remote(format!("{origin}/*"))
        .local(false)
        .window(label)
        .permission("allow-card-list")
        .permission("allow-card-sign")
        .permission("allow-card-close")
        .permission("allow-device-call")
        .permission("core:event:default");
    app.add_capability(capability)?;
    origins.insert(label.into());
    Ok(())
}

fn open_service(app: &AppHandle, label: &str, title: &str, url: Url) -> tauri::Result<()> {
    if app.get_webview_window(label).is_some() {
        return documents::focus(app, label);
    }
    WebviewWindowBuilder::new(app, label, WebviewUrl::External(url))
        .title(title).inner_size(1080.0, 760.0).build()?;
    Ok(())
}

fn refresh_menu(app: &AppHandle) -> tauri::Result<()> {
    app.set_menu(build_menu(app)?)?;
    Ok(())
}

fn choose_document(app: &AppHandle) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    return picker::show(app);
    #[cfg(not(target_os = "macos"))]
    open_setup(app)
}

fn open_setup(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(SETUP) {
        return window.set_focus();
    }
    WebviewWindowBuilder::new(app, SETUP, WebviewUrl::App("index.html".into()))
        .title("Configura il gestionale")
        .inner_size(520.0, 380.0)
        .build()?;
    Ok(())
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::default(app)?;
    let change_url = MenuItem::with_id(app, "change_url", "Apri RunDoc…", true, Some("CmdOrCtrl+Shift+U"))?;
    let reload = MenuItem::with_id(app, "reload", "Reload", true, Some("CmdOrCtrl+R"))?;
    let (docs, active) = {
        let state = app.state::<Registry>();
        let windows = state.lock().unwrap();
        (windows.open.clone(), windows.active.clone())
    };
    let current = active.as_ref().and_then(|id| docs.get(id));
    let instance = Submenu::with_id_and_icon(app, "documents", current.map_or("Gestionali", |d| &d.name), true, current.and_then(Document::image))?;
    for (label, doc) in docs {
        let title = if Some(&label) == active.as_ref() { format!("✓ {}", doc.name) } else { doc.name.clone() };
        instance.append(&tauri::menu::IconMenuItem::with_id(app, label, title, true, doc.image(), None::<&str>)?)?;
    }
    instance.append(&change_url)?;
    instance.append(&reload)?;
    menu.append(&instance)?;
    let team = MenuItem::with_id(app, "open_meetlot", "Riunioni di team", true, None::<&str>)?;
    menu.append(&Submenu::with_items(app, "Meetlot", true, &[&team])?)?;
    let support = MenuItem::with_id(app, "open_support", "Richiedi assistenza…", true, None::<&str>)?;
    menu.append(&Submenu::with_items(app, "Assistenza", true, &[&support])?)?;
    Ok(menu)
}

fn on_menu(app: &AppHandle, id: &str) -> tauri::Result<()> {
    match id {
        label if label.starts_with("doc-") => documents::focus(app, label),
        "open_meetlot" => open_service(app, MEETLOT, "Meetlot", Url::parse("https://meetlot.genro.com/call").expect("valid team URL")),
        "open_support" => {
            let mut url = Url::parse("https://meetlot.genro.com/support").expect("valid support URL");
            if let Some((_, doc)) = documents::active(app) {
                url.query_pairs_mut().append_pair("app", &doc.name);
            }
            open_service(app, SUPPORT, "Assistenza", url)
        }
        "change_url" => choose_document(app),
        "reload" => match documents::active(app).and_then(|(id, _)| app.get_webview_window(&id)) {
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
        let libraries = card::library_paths(&load_config(app)?.pkcs11_libraries)?;
        *guard = Some(Cards::load(&libraries)?);
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

#[derive(Default)]
struct PendingConnection(Mutex<Option<Result<String, String>>>);

fn is_connection_file(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("softwell") || ext.eq_ignore_ascii_case("rundoc"))
}

fn read_document(path: &std::path::Path) -> Result<Document, String> {
    if !path.is_file() || !is_connection_file(path) {
        return Err("Seleziona un RunDoc.".into());
    }
    let mut text = String::new();
    fs::File::open(path).and_then(|f| f.take(connector::MAX_SIZE as u64 + 1).read_to_string(&mut text))
        .map_err(|_| "Impossibile leggere il collegamento.")?;
    let link = connector::verify(&text)?;
    Ok(Document { name: link.name, url: link.url, icon: link.icon, id: link.id })
}

fn handle_document(app: &AppHandle, path: &std::path::Path) {
    let result = read_document(path).and_then(|doc| documents::open(app, doc).map_err(|e| e.to_string()));
    if let Err(error) = result {
        *app.state::<PendingConnection>().0.lock().unwrap() = Some(Err(error));
        if open_setup(app).is_ok() { let _ = app.emit_to(SETUP, "connection-opened", ()); }
    } else if let Some(window) = app.get_webview_window(SETUP) {
        let _ = window.close();
    }
}

#[tauri::command]
fn pending_connection(state: State<'_, PendingConnection>) -> Result<Option<String>, String> {
    state.0.lock().unwrap().clone().transpose()
}

#[tauri::command]
fn inspect_connection(text: String) -> Result<connector::Connection, String> {
    connector::verify(&text)
}

#[tauri::command]
async fn activate_connection(app: AppHandle, text: String) -> Result<(), String> {
    let link = connector::verify(&text)?;
    let doc = Document { url: link.url, name: link.name, id: link.id, icon: link.icon };
    documents::open(&app, doc).map_err(|e| e.to_string())?;
    *app.state::<PendingConnection>().0.lock().unwrap() = None;

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
    #[cfg(target_os = "macos")]
    let mut initial_picker = true;
    tauri::Builder::default()
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&[SETUP, PIN])
                .build(),
        )
        .manage(PendingConnection::default())
        .manage(Registry::default())
        .manage(Signer::default())
        .invoke_handler(tauri::generate_handler![
            pending_connection,
            inspect_connection,
            activate_connection,
            pin_request,
            pin_submit,
            pin_cancel,
            card_list,
            card_sign,
            card_close,
            device_call
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                if window.label() == SETUP || window.label().starts_with("doc-") {
                    for path in paths.iter().filter(|path| is_connection_file(path)) {
                        handle_document(window.app_handle(), path);
                    }
                }
            }
            if window.label().starts_with("doc-") {
                let registry = window.state::<Registry>();
                let mut docs = registry.lock().unwrap();
                let changed = match event {
                    WindowEvent::Focused(true) if docs.open.contains_key(window.label()) => {
                        docs.active = Some(window.label().into());
                        true
                    }
                    WindowEvent::Destroyed => {
                        docs.open.remove(window.label());
                        if docs.active.as_deref() == Some(window.label()) { docs.active = docs.open.keys().next().cloned(); }
                        true
                    }
                    _ => false,
                };
                drop(docs);
                if changed { let _ = refresh_menu(window.app_handle()); }
            }
            // Closing the PIN window with its close button cancels the request.
            if window.label() == PIN && matches!(event, WindowEvent::Destroyed) {
                if let Some(pending) = window.state::<Signer>().pending.lock().unwrap().take() {
                    let _ = pending.reply.send(Err("PIN cancelled".to_string()));
                }
            }
        })
        .setup(|app| {
            let handle = app.handle();
            let sidecar = Sidecar::start(handle.clone(), &config_path(handle)?)?;
            handle.manage(sidecar);
            handle.set_menu(build_menu(handle)?)?;
            handle.on_menu_event(|app, event| {
                if let Err(e) = on_menu(app, event.id().as_ref()) {
                    eprintln!("menu {}: {e}", event.id().as_ref());
                }
            });
            #[cfg(not(target_os = "macos"))]
            if let Some(path) = std::env::args_os().nth(1).map(PathBuf::from)
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("softwell") || e.eq_ignore_ascii_case("rundoc"))) {
                handle_document(handle, &path);
                return Ok(());
            }
            if let Some(url) = cli_url() {
                documents::open(handle, Document::from_url(&parse_url(&url)?))?;
            } else {
                #[cfg(not(target_os = "macos"))]
                choose_document(handle)?;
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the application")
        .run(move |app, event| {
            #[cfg(target_os = "macos")]
            match &event {
                RunEvent::Opened { urls } => {
                    initial_picker = false;
                    picker::dismiss();
                    for path in urls.iter().filter_map(|url| url.to_file_path().ok()) {
                        handle_document(app, &path);
                    }
                }
                RunEvent::MainEventsCleared if initial_picker => {
                    initial_picker = false;
                    if app.webview_windows().is_empty() { let _ = choose_document(app); }
                }
                RunEvent::Reopen { has_visible_windows: false, .. } if app.webview_windows().is_empty() => {
                    initial_picker = false;
                    let _ = choose_document(app);
                }
                RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
                _ => (),
            }
            if let RunEvent::Exit = event {
                app.state::<Sidecar>().stop();
            }
        });
}
