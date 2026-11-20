//! Card signing (PKCS#11), built with the feature `cards`: the card commands of the GenroPy page, the PIN window
//! and the menu "Firma digitale".
//!
//! The card commands are granted at runtime only to the windows of saved sites, for the origin of the site.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{UnparsedPublicKey, RSA_PKCS1_2048_8192_SHA256};
use sha2::{Digest, Sha256};
use x509_parser::parse_x509_certificate;
use tauri::ipc::CapabilityBuilder;
use tauri::menu::{MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, State, Url, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tokio::sync::oneshot;

use crate::card::{self, CardInfo, Cards, PinRequest};
use crate::{load_config, PIN};

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
    /// Window labels that already received the card capability.
    labels: Mutex<HashSet<String>>,
    /// The cards of the last read, shown by the menu "Firma digitale". `None` until the first read: the menu never
    /// loads the PKCS#11 libraries by itself.
    listed: Mutex<Option<Vec<CardInfo>>>,
}

/// Grants the card commands to the pages of `url`'s origin in the window `label`.
pub fn grant(app: &AppHandle, url: &Url, label: &str) -> tauri::Result<()> {
    let signer = app.state::<Signer>();
    let mut labels = signer.labels.lock().unwrap();
    if labels.contains(label) {
        return Ok(());
    }
    let origin = url.origin().ascii_serialization();
    let capability = CapabilityBuilder::new(format!("card-{}", labels.len()))
        .remote(format!("{origin}/*"))
        .local(false)
        .window(label)
        .permission("allow-card-list")
        .permission("allow-card-sign")
        .permission("allow-card-close");
    app.add_capability(capability)?;
    labels.insert(label.into());
    Ok(())
}

/// Runs `action` on the cards, loading the library and re-reading the readers first. Afterwards the menu
/// "Firma digitale" is rebuilt when the cards or their state changed.
fn with_cards<T>(app: &AppHandle, action: impl FnOnce(&mut Cards) -> Result<T, String>) -> Result<T, String> {
    let signer = app.state::<Signer>();
    let mut guard = signer.cards.lock().unwrap();
    if guard.is_none() {
        let libraries = card::library_paths(&load_config(app)?.pkcs11_libraries)?;
        *guard = Some(Cards::load(&libraries)?);
    }
    let cards = guard.as_mut().unwrap();
    cards.refresh()?;
    let result = action(cards);
    let mut listed = cards.list();
    listed.sort_by(|a, b| (&a.holder, &a.fingerprint).cmp(&(&b.holder, &b.fingerprint)));
    drop(guard);
    let changed = {
        let mut previous = signer.listed.lock().unwrap();
        let changed = previous.as_ref() != Some(&listed);
        *previous = Some(listed);
        changed
    };
    if changed {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Err(e) = crate::refresh_menu(&handle) {
                eprintln!("menu: {e}");
            }
        });
    }
    result
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

/// Signs a random message with `card` (PIN first when closed) and verifies the signature with the public key of
/// its certificate, here. Run by "Prova firma" of the menu "Firma digitale".
async fn test_card(app: &AppHandle, card: &str) -> Result<String, String> {
    let mut message = [0u8; 64];
    SystemRandom::new().fill(&mut message).map_err(|_| "random generator not available".to_string())?;
    let digest = Sha256::digest(message);
    if !with_cards(app, |cards| Ok(cards.is_open(card)))? {
        ask_pin(app, card).await?;
    }
    let signature = with_cards(app, |cards| cards.sign(card, &digest))?;
    let der = with_cards(app, |cards| cards.certificate(card))?;
    let (_, certificate) = parse_x509_certificate(&der).map_err(|e| format!("certificate: {e}"))?;
    let public_key = &certificate.public_key().subject_public_key.data;
    UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, public_key.as_ref())
        .verify(&message, &signature)
        .map_err(|_| "La firma non corrisponde al certificato della carta.".to_string())?;
    Ok(format!("Firma verificata con il certificato di {}.", certificate.subject()))
}

#[tauri::command]
pub async fn card_close(app: AppHandle, card: String) -> Result<(), String> {
    with_cards(&app, |cards| cards.close(&card))
}

// ------------------------------------------------------------------ menu "Firma digitale"

/// The menu "Firma digitale": one submenu per card of the last read, with "Prova firma" and "Chiudi carta", and
/// "Leggi i lettori". It shows the cards read last and does not read the readers itself.
pub fn menu(app: &AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let menu = Submenu::new(app, "Firma digitale", true)?;
    let listed = app.state::<Signer>().listed.lock().unwrap().clone();
    if let Some(listed) = &listed {
        if listed.is_empty() {
            menu.append(&MenuItem::new(app, "Nessuna carta nei lettori", false, None::<&str>)?)?;
        }
        for card in listed {
            let state = if card.open { "aperta" } else { "chiusa" };
            let title = format!("{} – scade {} ({state})", card.holder, card.valid_until);
            let id = &card.fingerprint;
            menu.append(&Submenu::with_items(app, title, true, &[
                &MenuItem::new(app, format!("Numero di serie {}", card.serial), false, None::<&str>)?,
                &PredefinedMenuItem::separator(app)?,
                &MenuItem::with_id(app, format!("card_test:{id}"), "Prova firma", true, None::<&str>)?,
                &MenuItem::with_id(app, format!("card_close:{id}"), "Chiudi carta", card.open, None::<&str>)?,
            ])?)?;
        }
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    let read = if listed.is_some() { "Rileggi i lettori" } else { "Leggi i lettori" };
    menu.append(&MenuItem::with_id(app, "cards_read", read, true, None::<&str>)?)?;
    Ok(menu)
}

/// Handles an item of the menu "Firma digitale"; returns false when `id` is not one of its items.
/// The card work runs off the main thread; its result, or its error, is shown in a message.
pub fn on_menu(app: &AppHandle, id: &str) -> bool {
    let action = if id == "cards_read" {
        Action::Read
    } else if let Some(card) = id.strip_prefix("card_test:") {
        Action::Test(card.to_string())
    } else if let Some(card) = id.strip_prefix("card_close:") {
        Action::Close(card.to_string())
    } else {
        return false;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = match action {
            Action::Read => with_cards(&app, |cards| Ok(cards.list().len())).map(|count| match count {
                0 => "Nessuna carta nei lettori.".to_string(),
                1 => "Una carta nei lettori: è nel menu Firma digitale.".to_string(),
                n => format!("{n} carte nei lettori: sono nel menu Firma digitale."),
            }),
            Action::Test(card) => test_card(&app, &card).await,
            Action::Close(card) => with_cards(&app, |cards| cards.close(&card)).map(|_| "Carta chiusa.".to_string()),
        };
        let (text, kind) = match result {
            Ok(text) => (text, MessageDialogKind::Info),
            Err(text) => (text, MessageDialogKind::Error),
        };
        app.dialog().message(text).title("Firma digitale").kind(kind).show(|_| {});
    });
    true
}

enum Action {
    Read,
    Test(String),
    Close(String),
}

/// Registers the card state.
pub fn setup(app: &AppHandle) {
    app.manage(Signer::default());
}

/// Closing the PIN window with its close button cancels the request.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() == PIN && matches!(event, WindowEvent::Destroyed) {
        if let Some(pending) = window.state::<Signer>().pending.lock().unwrap().take() {
            let _ = pending.reply.send(Err("PIN cancelled".to_string()));
        }
    }
}
