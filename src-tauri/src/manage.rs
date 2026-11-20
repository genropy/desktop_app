//! The window "Gestione": the saved sites and a local test of the device key. The signature cards are in the
//! menu "Firma digitale" (`cards.rs`).
//!
//! Its commands are granted only to this local window (`capabilities/manage.json`), never to a site page.
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::device_key;
use crate::sites;

pub const WINDOW: &str = "manage";
/// First line of the test message: it differs from the one of a login, so a test signature never logs in.
const TEST_DOMAIN: &str = "genroapp-device-test";

pub fn open(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(WINDOW) {
        window.unminimize()?;
        return window.set_focus();
    }
    WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("manage.html".into()))
        .title("Gestione")
        .inner_size(640.0, 520.0)
        .build()?;
    Ok(())
}

#[derive(Serialize)]
pub struct Features {
    platform: &'static str,
    /// Downloaded Python modules: only in builds with the feature `sidecar`.
    modules: bool,
}

#[tauri::command]
pub fn manage_features() -> Features {
    Features { platform: device_key::PLATFORM, modules: cfg!(feature = "sidecar") }
}

#[derive(Serialize)]
pub struct SiteRow {
    name: String,
    origin: String,
    user: Option<String>,
    verification: Option<String>,
    primary: bool,
    /// A key of this computer exists for the site.
    registered: bool,
}

#[tauri::command]
pub async fn manage_sites(app: AppHandle) -> Result<Vec<SiteRow>, String> {
    let saved = sites::load(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        saved
            .into_iter()
            .map(|site| {
                let origin = site.origin();
                let registered = site.key_id.is_some() && device_key::exists(&device_key::label(&origin));
                SiteRow { name: site.name, origin, user: site.user, verification: site.verification, primary: site.primary, registered }
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn manage_set_primary(app: AppHandle, origin: String) -> Result<(), String> {
    sites::set_primary(&app, &origin)?;
    crate::refresh_menu(&app).map_err(|e| e.to_string())
}

/// Deletes the key of this computer for `origin` and forgets the user. The site keeps its row of the device until
/// an administrator revokes it.
#[tauri::command]
pub async fn manage_forget(app: AppHandle, origin: String) -> Result<(), String> {
    crate::device_login::forget(&app, &origin).await
}

/// Removes the site of `origin` from GenroApp, with its key.
#[tauri::command]
pub async fn manage_remove(app: AppHandle, origin: String) -> Result<(), String> {
    let label = device_key::label(&origin);
    tauri::async_runtime::spawn_blocking(move || device_key::delete(&label)).await.map_err(|e| e.to_string())??;
    crate::fiscal::forget(&app, &origin)?;
    sites::remove(&app, &origin)?;
    crate::refresh_menu(&app).map_err(|e| e.to_string())
}

/// The message a test signature covers.
fn test_message(origin: &str, nonce: &[u8]) -> Vec<u8> {
    let mut message = format!("{TEST_DOMAIN}\n{origin}\n").into_bytes();
    message.extend_from_slice(nonce);
    message
}

/// Signs a random message with the key of `origin` (Touch ID, or the password per the key) and verifies the
/// signature with the public key, here. The site is not involved.
#[tauri::command]
pub async fn manage_test_key(app: AppHandle, origin: String) -> Result<String, String> {
    let site = sites::find(&app, &origin)?.ok_or("Sito non trovato.")?;
    let label = device_key::label(&origin);
    let reason = format!("provare la chiave di {}", site.name);
    let mut nonce = [0u8; 16];
    SystemRandom::new().fill(&mut nonce).map_err(|_| "Generatore casuale non disponibile.")?;
    let message = test_message(&origin, &nonce);
    let signed = message.clone();
    let (point, signature) = tauri::async_runtime::spawn_blocking(move || {
        let point = device_key::public_point(&label)?;
        let signature = device_key::sign(&label, &reason, &signed)?;
        Ok::<_, String>((point, signature))
    })
    .await
    .map_err(|e| e.to_string())??;
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &point)
        .verify(&message, &signature)
        .map_err(|_| "La firma non corrisponde alla chiave pubblica.")?;
    let key_id = crate::device_login::key_id(&crate::device_login::spki_from_point(&point)?);
    let stored = site.key_id.as_deref() == Some(key_id.as_str());
    Ok(if stored {
        format!("Firma verificata con la chiave {key_id}.")
    } else {
        format!("Firma verificata, ma la chiave {key_id} non è quella registrata per il sito.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_signature_never_covers_a_login_message() {
        let test = test_message("https://localhost:8080", b"abc");
        assert!(test.starts_with(b"genroapp-device-test\n"));
        let login = crate::device_login::signed_bytes("https://localhost:8080", "mario", "abc").unwrap();
        assert!(!login.starts_with(TEST_DOMAIN.as_bytes()));
        assert_ne!(test, login);
    }
}
