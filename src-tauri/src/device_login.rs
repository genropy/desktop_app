//! The commands the page of a saved site calls to register this computer and to sign a login challenge.
//!
//! They are granted at runtime only to the windows of saved sites, for the origin of the site. Each command also
//! checks that the `origin` it receives is the origin of the calling page and of a saved site.
//!
//! Encodings (shared with GenroPy):
//! - `public_key`: base64 of the SubjectPublicKeyInfo DER;
//! - `signature`: base64 of the ECDSA P-256 SHA-256 signature, DER;
//! - `key_id`: base64url without padding of SHA-256 of the SubjectPublicKeyInfo DER;
//! - signed bytes: `genroapp-device-v1\n<origin>\n<user>\n<challenge>`, UTF-8, no trailing newline.
use std::collections::HashSet;
use std::sync::Mutex;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::ipc::CapabilityBuilder;
use tauri::{AppHandle, Manager, Url, Webview};

use crate::device_key;
use crate::sites::{self, Site};

const DOMAIN: &str = "genroapp-device-v1";
const ALG: &str = "ES256";
/// DER prefix of a SubjectPublicKeyInfo for an uncompressed P-256 point (id-ecPublicKey, prime256v1).
const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
    0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// Window labels that already received the device commands.
#[derive(Default)]
pub struct Grants(Mutex<HashSet<String>>);

/// Grants the device commands to the pages of `url`'s origin in the window `label`.
pub fn grant(app: &AppHandle, url: &Url, label: &str) -> tauri::Result<()> {
    let grants = app.state::<Grants>();
    let mut labels = grants.0.lock().unwrap();
    if labels.contains(label) {
        return Ok(());
    }
    let origin = url.origin().ascii_serialization();
    let capability = CapabilityBuilder::new(format!("device-login-{}", labels.len()))
        .remote(format!("{origin}/*"))
        .local(false)
        .window(label)
        .permission("allow-device-status")
        .permission("allow-device-register")
        .permission("allow-device-sign")
        .permission("allow-device-forget");
    app.add_capability(capability)?;
    labels.insert(label.into());
    Ok(())
}

/// SubjectPublicKeyInfo DER of an uncompressed P-256 point (65 bytes, `0x04 || x || y`).
pub fn spki_from_point(point: &[u8]) -> Result<Vec<u8>, String> {
    if point.len() != 65 || point[0] != 0x04 {
        return Err("La chiave pubblica non è un punto P-256.".into());
    }
    Ok([&P256_SPKI_PREFIX[..], point].concat())
}

pub fn key_id(spki: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(spki))
}

/// The exact bytes signed for `origin`, `user` and `challenge`. A newline in a field would change the lines,
/// so control characters are refused; the challenge must be base64url without padding.
pub fn signed_bytes(origin: &str, user: &str, challenge: &str) -> Result<Vec<u8>, String> {
    if user.is_empty() || user.chars().any(char::is_control) {
        return Err("Nome utente non valido.".into());
    }
    if challenge.is_empty() || !challenge.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err("Challenge non valida.".into());
    }
    if origin.chars().any(char::is_control) {
        return Err("Origine non valida.".into());
    }
    Ok(format!("{DOMAIN}\n{origin}\n{user}\n{challenge}").into_bytes())
}

/// The saved site of `origin`, when `origin` is also the origin of the calling page.
pub(crate) fn caller_site(app: &AppHandle, webview: &Webview, origin: &str) -> Result<Site, String> {
    let page = webview.url().map_err(|e| e.to_string())?.origin().ascii_serialization();
    if page != origin {
        return Err(format!("L'origine {origin} non è quella della pagina."));
    }
    sites::find(app, origin)?.ok_or_else(|| format!("Il sito {origin} non è fra quelli aggiunti a GenroApp."))
}

/// Runs the blocking `action` (it waits for Touch ID) outside the async runtime threads.
async fn blocking<T: Send + 'static>(action: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(action).await.map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct Status {
    registered: bool,
    user: Option<String>,
    key_id: Option<String>,
    platform: &'static str,
}

#[derive(Serialize)]
pub struct Registration {
    key_id: String,
    public_key: String,
    alg: &'static str,
    device_name: String,
    verification: &'static str,
    signature: String,
}

#[derive(Serialize)]
pub struct Signature {
    key_id: String,
    user: String,
    signature: String,
}

#[tauri::command]
pub async fn device_status(app: AppHandle, webview: Webview, origin: String) -> Result<Status, String> {
    let site = caller_site(&app, &webview, &origin)?;
    let label = device_key::label(&origin);
    let registered = site.user.is_some() && site.key_id.is_some() && blocking(move || Ok(device_key::exists(&label))).await?;
    Ok(Status {
        registered,
        user: site.user.filter(|_| registered),
        key_id: site.key_id.filter(|_| registered),
        platform: device_key::PLATFORM,
    })
}

/// Creates the key of this computer for `origin` and `user` (replacing an earlier one) and signs `challenge`.
#[tauri::command]
pub async fn device_register(
    app: AppHandle,
    webview: Webview,
    origin: String,
    user: String,
    challenge: String,
) -> Result<Registration, String> {
    let site = caller_site(&app, &webview, &origin)?;
    let message = signed_bytes(&origin, &user, &challenge)?;
    let label = device_key::label(&origin);
    let reason = format!("registrare questo computer su {} come {user}", site.name);
    let (point, verification, signature) = blocking(move || {
        device_key::delete(&label)?;
        let (point, verification) = device_key::create(&label)?;
        match device_key::sign(&label, &reason, &message) {
            Ok(signature) => Ok((point, verification, signature)),
            Err(e) => {
                let _ = device_key::delete(&label);
                Err(e)
            }
        }
    })
    .await?;
    let spki = spki_from_point(&point)?;
    let key_id = key_id(&spki);
    sites::update(&app, &origin, |site| {
        site.user = Some(user);
        site.key_id = Some(key_id.clone());
        site.verification = Some(verification.as_str().into());
    })?;
    Ok(Registration {
        key_id,
        public_key: STANDARD.encode(&spki),
        alg: ALG,
        device_name: device_key::device_name(),
        verification: verification.as_str(),
        signature: STANDARD.encode(signature),
    })
}

/// Signs `challenge` with the key of this computer for `origin`, after Touch ID.
#[tauri::command]
pub async fn device_sign(app: AppHandle, webview: Webview, origin: String, challenge: String) -> Result<Signature, String> {
    let site = caller_site(&app, &webview, &origin)?;
    let (Some(user), Some(key_id)) = (site.user.clone(), site.key_id.clone()) else {
        return Err("Questo computer non è registrato sul sito.".into());
    };
    let message = signed_bytes(&origin, &user, &challenge)?;
    let label = device_key::label(&origin);
    let reason = format!("accedere a {} come {user}", site.name);
    let signature = blocking(move || device_key::sign(&label, &reason, &message)).await?;
    Ok(Signature { key_id, user, signature: STANDARD.encode(signature) })
}

/// Deletes the key of this computer for `origin` and forgets the user.
pub async fn forget(app: &AppHandle, origin: &str) -> Result<(), String> {
    let label = device_key::label(origin);
    blocking(move || device_key::delete(&label)).await?;
    sites::update(app, origin, |site| {
        site.user = None;
        site.key_id = None;
        site.verification = None;
    })?;
    Ok(())
}

#[tauri::command]
pub async fn device_forget(app: AppHandle, webview: Webview, origin: String) -> Result<(), String> {
    caller_site(&app, &webview, &origin)?;
    forget(&app, &origin).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1, ECDSA_P256_SHA256_ASN1_SIGNING};

    #[test]
    fn the_signed_bytes_are_the_four_lines_of_the_contract() {
        let bytes = signed_bytes("https://app.acme.com", "mario", "q0-_Zz9").unwrap();
        assert_eq!(bytes, b"genroapp-device-v1\nhttps://app.acme.com\nmario\nq0-_Zz9");
        assert!(signed_bytes("https://app.acme.com", "mario\nadmin", "abc").is_err());
        assert!(signed_bytes("https://app.acme.com", "", "abc").is_err());
        assert!(signed_bytes("https://app.acme.com", "mario", "abc=").is_err());
        assert!(signed_bytes("https://app.acme.com", "mario", "a+b/").is_err());
        assert!(signed_bytes("https://app.acme.com", "mario", "").is_err());
    }

    /// A P-256 key in software stands for the Secure Enclave: the site must verify what the app sends.
    #[test]
    fn the_site_verifies_the_signature_with_the_public_key_sent() {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
        let spki = spki_from_point(pair.public_key().as_ref()).unwrap();
        assert_eq!(spki.len(), 91);

        let challenge = URL_SAFE_NO_PAD.encode([7u8; 32]);
        let message = signed_bytes("https://localhost:8080", "mario", &challenge).unwrap();
        let signature = STANDARD.encode(pair.sign(&rng, &message).unwrap());

        // What GenroPy does: decode the SPKI, take the point, verify the DER signature over the same bytes.
        let received = STANDARD.decode(STANDARD.encode(&spki)).unwrap();
        assert_eq!(&received[..26], &P256_SPKI_PREFIX);
        let public = UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &received[26..]);
        assert!(public.verify(&message, &STANDARD.decode(&signature).unwrap()).is_ok());
        let other = signed_bytes("https://localhost:8080", "luigi", &challenge).unwrap();
        assert!(public.verify(&other, &STANDARD.decode(&signature).unwrap()).is_err());
    }

    #[test]
    fn the_key_id_is_unpadded_base64url_of_the_spki_hash() {
        let spki = spki_from_point(&[[4u8].as_slice(), &[1u8; 64]].concat()).unwrap();
        let id = key_id(&spki);
        assert_eq!(id.len(), 43);
        assert!(!id.contains('=') && !id.contains('+') && !id.contains('/'));
        assert_eq!(URL_SAFE_NO_PAD.decode(&id).unwrap(), Sha256::digest(&spki).to_vec());
        assert!(spki_from_point(&[4u8; 64]).is_err());
        assert!(spki_from_point(&[[2u8].as_slice(), &[1u8; 64]].concat()).is_err());
    }
}
