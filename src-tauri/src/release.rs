//! The Tauri updater and the About window.
//!
//! At start and every hour a release build reads `latest.json` from the downloads bucket. A newer signed release
//! puts a badge on the Dock icon and the item "Aggiorna a <version> e riavvia" in the GenroApp menu; the item
//! downloads the release, verifies its signature with the public key in `tauri.conf.json`, installs it and
//! restarts.
use std::sync::Mutex;
use std::time::Duration;

use tauri::menu::{MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_updater::{Update, UpdaterExt};

/// How often a running app looks for a newer release.
const CHECK_EVERY: Duration = Duration::from_secs(3600);

/// The release found and not yet installed.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

fn pending_version(app: &AppHandle) -> Option<String> {
    app.state::<Pending>().0.lock().unwrap().as_ref().map(|update| update.version.clone())
}

/// Shows or clears the badge on the Dock icon and rebuilds the menu with or without the update item.
fn show_update(app: &AppHandle, version: Option<&str>) {
    #[cfg(target_os = "macos")]
    {
        let label = version.map(|_| "1");
        let _ = app.run_on_main_thread(move || {
            let mtm = objc2_foundation::MainThreadMarker::new().expect("main thread");
            let tile = objc2_app_kit::NSApplication::sharedApplication(mtm).dockTile();
            tile.setBadgeLabel(label.map(objc2_foundation::NSString::from_str).as_deref());
        });
    }
    if let Err(error) = crate::refresh_menu(app) {
        eprintln!("GenroApp update: {error}");
    }
}

/// At start and every hour: a newer signed release is kept in `Pending` and offered in the menu.
#[cfg_attr(debug_assertions, allow(dead_code))]
pub fn watch_updates(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            match app.updater() {
                Ok(updater) => match updater.check().await {
                    Ok(Some(update)) => {
                        let version = update.version.clone();
                        *app.state::<Pending>().0.lock().unwrap() = Some(update);
                        show_update(&app, Some(&version));
                    }
                    Ok(None) => (),
                    Err(error) => eprintln!("GenroApp update: {error}"),
                },
                Err(error) => eprintln!("GenroApp update: {error}"),
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// The menu item "Aggiorna a <version> e riavvia": download, verify the signature, install, restart.
pub fn install(app: &AppHandle) {
    let Some(update) = app.state::<Pending>().0.lock().unwrap().take() else { return };
    show_update(app, None);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match update.download_and_install(|_, _| {}, || {}).await {
            Ok(()) => app.restart(),
            Err(error) => {
                eprintln!("GenroApp update: {error}");
                let version = update.version.clone();
                *app.state::<Pending>().0.lock().unwrap() = Some(update);
                show_update(&app, Some(&version));
            }
        }
    });
}

/// The GenroApp menu: About GenroApp opens its own window instead of the standard panel, and a pending release adds
/// "Aggiorna a <version> e riavvia" below it.
pub fn app_menu(app: &AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let name = app.package_info().name.clone();
    let menu = Submenu::with_items(app, &name, true, &[
        &MenuItem::with_id(app, "about", format!("Informazioni su {name}"), true, None::<&str>)?,
        &PredefinedMenuItem::separator(app)?,
        &MenuItem::with_id(app, "manage", "Gestione…", true, Some("CmdOrCtrl+,"))?,
    ])?;
    if let Some(version) = pending_version(app) {
        let title = format!("Aggiorna a {version} e riavvia");
        menu.append(&MenuItem::with_id(app, "install_update", title, true, None::<&str>)?)?;
    }
    menu.append_items(&[
        &PredefinedMenuItem::separator(app)?,
        &PredefinedMenuItem::hide(app, Some(&format!("Nascondi {name}")))?,
        &PredefinedMenuItem::hide_others(app, Some("Nascondi altre"))?,
        &PredefinedMenuItem::separator(app)?,
        &PredefinedMenuItem::quit(app, Some(&format!("Esci da {name}")))?,
    ])?;
    Ok(menu)
}

/// About GenroApp: the version, what GenroApp is made with and the copyright. Its page is part of the app
/// (`setup/about.html`); links leave for the default browser.
pub fn open_about(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("about") {
        window.unminimize()?;
        return window.set_focus();
    }
    let version = serde_json::to_string(&format!("Versione {}", app.package_info().version)).unwrap();
    WebviewWindowBuilder::new(app, "about", WebviewUrl::App("about.html".into()))
        .title("Informazioni su GenroApp")
        .inner_size(400.0, 380.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .on_navigation(|url| {
            if url.scheme() == "https" {
                let _ = std::process::Command::new("/usr/bin/open").arg(url.as_str()).spawn();
                return false;
            }
            true
        })
        .on_page_load(move |webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                let _ = webview.eval(format!("document.getElementById('version').textContent = {version}"));
            }
        })
        .build()?;
    Ok(())
}
