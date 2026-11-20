#[cfg(feature = "devices")]
mod card;
mod connector;
mod device_key;
mod device_login;
#[cfg(feature = "devices")]
mod devices;
mod documents;
mod fiscal;
mod local;
mod manage;
use documents::{Document, Registry};
#[cfg(target_os = "macos")]
mod picker;
mod release;
#[cfg(feature = "devices")]
mod sidecar;
mod sites;

use std::fs;
use std::io::Read;
#[cfg(feature = "devices")]
use std::io::ErrorKind;
#[cfg(any(feature = "devices", not(target_os = "macos")))]
use std::path::PathBuf;
use std::sync::Mutex;

#[cfg(feature = "devices")]
use serde::{Deserialize, Serialize};
use tauri::menu::{IconMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
#[cfg(feature = "devices")]
use serde_json::Value;
use tauri::{Emitter, AppHandle, Manager, RunEvent, State, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};

const SETUP: &str = "setup";
const PIN: &str = "pin";

/// Persisted in `<app config dir>/config.json`.
#[cfg(feature = "devices")]
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

#[cfg(feature = "devices")]
fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

#[cfg(feature = "devices")]
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

/// The window shown when GenroApp has none open: see `sites::start`.
fn first_window(app: &AppHandle) -> tauri::Result<()> {
    sites::open_start(app)
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

fn open_local_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(local::WINDOW) {
        return window.set_focus();
    }
    WebviewWindowBuilder::new(app, local::WINDOW, WebviewUrl::App("local.html".into()))
        .title("Apri istanza locale")
        .inner_size(380.0, 190.0)
        .resizable(false)
        .build()?;
    Ok(())
}

/// The menu bar: GenroApp, Modifica (the shortcuts of the page need it) and Finestre.
fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let edit = Submenu::with_items(app, "Modifica", true, &[
        &PredefinedMenuItem::undo(app, Some("Annulla"))?,
        &PredefinedMenuItem::redo(app, Some("Ripeti"))?,
        &PredefinedMenuItem::separator(app)?,
        &PredefinedMenuItem::cut(app, Some("Taglia"))?,
        &PredefinedMenuItem::copy(app, Some("Copia"))?,
        &PredefinedMenuItem::paste(app, Some("Incolla"))?,
        &PredefinedMenuItem::select_all(app, Some("Seleziona tutto"))?,
    ])?;
    let windows = Submenu::with_id(app, "documents", "Finestre", true)?;
    let (docs, active) = {
        let state = app.state::<Registry>();
        let registry = state.lock().unwrap();
        (registry.open.clone(), registry.active.clone())
    };
    let has_docs = !docs.is_empty();
    for (label, doc) in docs {
        let title = if Some(&label) == active.as_ref() { format!("✓ {}", doc.name) } else { doc.name.clone() };
        windows.append(&IconMenuItem::with_id(app, label, title, true, doc.image(), None::<&str>)?)?;
    }
    let open_origins: Vec<String> = docs_origins(app);
    let saved: Vec<sites::Site> = sites::load(app)
        .unwrap_or_else(|e| {
            eprintln!("sites: {e}");
            Vec::new()
        })
        .into_iter()
        .filter(|site| !open_origins.contains(&site.origin()))
        .collect();
    for site in &saved {
        let id = format!("site:{}", site.origin());
        windows.append(&IconMenuItem::with_id(app, id, &site.name, true, site.document().image(), None::<&str>)?)?;
    }
    if has_docs || !saved.is_empty() {
        windows.append(&PredefinedMenuItem::separator(app)?)?;
    }
    let active_site = active.as_ref()
        .and_then(|label| docs_by_label(app, label))
        .and_then(|origin| sites::find(app, &origin).ok().flatten());
    if let Some(site) = active_site {
        let title = if site.primary {
            format!("Togli «{}» come sito primario", site.name)
        } else {
            format!("Imposta «{}» come sito primario", site.name)
        };
        windows.append(&MenuItem::with_id(app, format!("primary:{}", site.origin()), title, true, None::<&str>)?)?;
    }
    windows.append_items(&[
        &MenuItem::with_id(app, "add_site", "Aggiungi sito…", true, None::<&str>)?,
        &MenuItem::with_id(app, "change_url", "Apri RunDoc…", true, Some("CmdOrCtrl+Shift+U"))?,
        &MenuItem::with_id(app, "open_local", "Apri istanza locale…", true, Some("CmdOrCtrl+Shift+L"))?,
        &MenuItem::with_id(app, "reload", "Ricarica", true, Some("CmdOrCtrl+R"))?,
        &PredefinedMenuItem::separator(app)?,
        &PredefinedMenuItem::minimize(app, Some("Riduci a icona"))?,
        &PredefinedMenuItem::close_window(app, Some("Chiudi finestra"))?,
    ])?;
    Menu::with_items(app, &[&release::app_menu(app)?, &edit, &windows])
}

/// The origin of the document window `label`.
fn docs_by_label(app: &AppHandle, label: &str) -> Option<String> {
    let state = app.state::<Registry>();
    let registry = state.lock().unwrap();
    let url = Url::parse(&registry.open.get(label)?.url).ok()?;
    Some(url.origin().ascii_serialization())
}

/// The origins of the open document windows.
fn docs_origins(app: &AppHandle) -> Vec<String> {
    let state = app.state::<Registry>();
    let registry = state.lock().unwrap();
    registry
        .open
        .values()
        .filter_map(|doc| Url::parse(&doc.url).ok())
        .map(|url| url.origin().ascii_serialization())
        .collect()
}

fn on_menu(app: &AppHandle, id: &str) -> tauri::Result<()> {
    match id {
        label if label.starts_with("doc-") => documents::focus(app, label),
        id if id.starts_with("site:") => {
            sites::open_saved(app, id["site:".len()..].to_string());
            Ok(())
        }
        "add_site" => sites::open_window(app),
        id if id.starts_with("primary:") => {
            if let Err(e) = sites::set_primary(app, &id["primary:".len()..]) {
                eprintln!("primary site: {e}");
            }
            refresh_menu(app)
        }
        "about" => release::open_about(app),
        "manage" => manage::open(app),
        "install_update" => {
            release::install(app);
            Ok(())
        }
        "change_url" => choose_document(app),
        "open_local" => open_local_window(app),
        "reload" => match documents::active(app).and_then(|(id, _)| app.get_webview_window(&id)) {
            Some(window) => window.eval("window.location.reload()"),
            None => Ok(()),
        },
        _ => Ok(()),
    }
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

#[tauri::command]
async fn open_local(app: AppHandle, port: u16) -> Result<(), String> {
    documents::open(&app, local::document(port)?).map_err(|e| e.to_string())?;
    if let Some(window) = app.get_webview_window(local::WINDOW) {
        window.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "macos")]
    let mut initial_picker = true;
    tauri::Builder::default()
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&[SETUP, PIN, local::WINDOW, sites::WINDOW, sites::CHOOSER, manage::WINDOW])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .manage(PendingConnection::default())
        .manage(Registry::default())
        .manage(release::Pending::default())
        .manage(sites::SiteError::default())
        .manage(device_login::Grants::default())
        .manage(fiscal::Registers::default())
        .invoke_handler(tauri::generate_handler![
            pending_connection,
            inspect_connection,
            activate_connection,
            open_local,
            sites::add_site,
            sites::site_error,
            sites::saved_sites,
            sites::open_sites,
            manage::manage_features,
            manage::manage_sites,
            manage::manage_set_primary,
            manage::manage_forget,
            manage::manage_remove,
            manage::manage_test_key,
            #[cfg(feature = "devices")]
            devices::card_test,
            device_login::device_status,
            device_login::device_register,
            device_login::device_sign,
            device_login::device_forget,
            #[cfg(feature = "devices")]
            devices::pin_request,
            #[cfg(feature = "devices")]
            devices::pin_submit,
            #[cfg(feature = "devices")]
            devices::pin_cancel,
            #[cfg(feature = "devices")]
            devices::card_list,
            #[cfg(feature = "devices")]
            devices::card_sign,
            #[cfg(feature = "devices")]
            devices::card_close,
            #[cfg(feature = "devices")]
            devices::device_call,
            fiscal::fiscal_status,
            fiscal::fiscal_receipt,
            fiscal::fiscal_x_report,
            fiscal::fiscal_daily_closure,
            fiscal::manage_fiscal_get,
            fiscal::manage_fiscal_set,
            fiscal::manage_fiscal_test
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
            #[cfg(feature = "devices")]
            devices::on_window_event(window, event);
        })
        .setup(|app| {
            let handle = app.handle();
            #[cfg(feature = "devices")]
            devices::setup(handle)?;
            #[cfg(not(debug_assertions))]
            {
                handle.plugin(tauri_plugin_updater::Builder::new().build())?;
                release::watch_updates(handle.clone());
            }
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
                first_window(handle)?;
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
                    if app.webview_windows().is_empty() { let _ = first_window(app); }
                }
                RunEvent::Reopen { has_visible_windows: false, .. } if app.webview_windows().is_empty() => {
                    initial_picker = false;
                    let _ = first_window(app);
                }
                RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
                _ => (),
            }
            #[cfg(feature = "devices")]
            if let RunEvent::Exit = event {
                devices::stop(app);
            }
        });
}
