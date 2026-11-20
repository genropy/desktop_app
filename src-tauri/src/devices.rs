//! Card signing (PKCS#11) and the Python sidecar, built with the feature `devices`.
//!
//! The distributed site runner is built without this module: no PIN window, no card commands,
//! no sidecar process.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::Value;
use tauri::ipc::CapabilityBuilder;
use tauri::{AppHandle, Manager, State, Url, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};
use tokio::sync::oneshot;

use crate::card::{self, CardInfo, Cards, PinRequest};
use crate::sidecar::Sidecar;
use crate::{config_path, load_config, PIN};

const SESSION_MINUTES: u64 = 240;

/// A `card_sign` waiting for the PIN window.
pub struct PendingPin {
    fingerprint: String,
    reply: oneshot::Sender<Result<(), String>>,
}

#[derive(Default)]
pub struct Signer {
    /// Loaded on the first card command, so the runner works without the library.
    cards: Mutex<Option<Cards>>,
    pending: Mutex<Option<PendingPin>>,
    /// Origins that already received the card capability.
    origins: Mutex<HashSet<String>>,
}

/// Grants the card commands to pages of the origin of `url` in the main window.
/// A grant stays until the app exits, also after Change URL.
pub fn grant_card_commands(app: &AppHandle, url: &Url, label: &str) -> tauri::Result<()> {
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

// ------------------------------------------------------------------ PIN window

#[tauri::command]
pub async fn pin_request(app: AppHandle, signer: State<'_, Signer>) -> Result<PinRequest, String> {
    let fingerprint = pending_fingerprint(&signer)?;
    with_cards(&app, |cards| cards.pin_request(&fingerprint))
}

/// One login attempt. On error the window stays open and shows the message.
#[tauri::command]
pub async fn pin_submit(app: AppHandle, signer: State<'_, Signer>, pin: String) -> Result<(), String> {
    let fingerprint = pending_fingerprint(&signer)?;
    let minutes = load_config(&app)?.session_minutes.unwrap_or(SESSION_MINUTES);
    let until = SystemTime::now() + Duration::from_secs(minutes * 60);
    with_cards(&app, |cards| cards.open(&fingerprint, &pin, until))?;
    finish_pin(&app, Ok(()))
}

#[tauri::command]
pub async fn pin_cancel(app: AppHandle) -> Result<(), String> {
    finish_pin(&app, Err("PIN cancelled".to_string()))
}

// ------------------------------------------------------------------ GenroPy page

#[tauri::command]
pub async fn card_list(app: AppHandle) -> Result<Vec<CardInfo>, String> {
    with_cards(&app, |cards| Ok(cards.list()))
}

/// Signature (base64) of `digest` (base64 of 32 bytes) with the card `card`.
/// A closed card first asks its PIN in a local window: the page never sees the PIN.
#[tauri::command]
pub async fn card_sign(app: AppHandle, card: String, digest: String) -> Result<String, String> {
    let digest = STANDARD.decode(digest.trim()).map_err(|e| format!("digest is not base64: {e}"))?;
    if !with_cards(&app, |cards| Ok(cards.is_open(&card)))? {
        ask_pin(&app, &card).await?;
    }
    let signature = with_cards(&app, |cards| cards.sign(&card, &digest))?;
    Ok(STANDARD.encode(signature))
}

#[tauri::command]
pub async fn card_close(app: AppHandle, card: String) -> Result<(), String> {
    with_cards(&app, |cards| cards.close(&card))
}

// ------------------------------------------------------------------ Python sidecar

/// Calls `module.method(**params)` in the Python sidecar.
#[tauri::command]
pub async fn device_call(
    sidecar: State<'_, Sidecar>,
    module: String,
    method: String,
    params: Option<Value>,
) -> Result<Value, String> {
    sidecar.call(&module, &method, params.unwrap_or(Value::Null)).await
}

/// Starts the sidecar and registers the card state.
pub fn setup(app: &AppHandle) -> Result<(), String> {
    app.manage(Signer::default());
    app.manage(Sidecar::start(app.clone(), &config_path(app)?)?);
    Ok(())
}

/// Closing the PIN window with its close button cancels the request.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() == PIN && matches!(event, WindowEvent::Destroyed) {
        if let Some(pending) = window.state::<Signer>().pending.lock().unwrap().take() {
            let _ = pending.reply.send(Err("PIN cancelled".to_string()));
        }
    }
}

pub fn stop(app: &AppHandle) {
    app.state::<Sidecar>().stop();
}
