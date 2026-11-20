use base64::{engine::general_purpose::STANDARD, Engine};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use tauri::Url;

pub const MAX_SIZE: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    payload: String,
    signature: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    format: String,
    pub name: String,
    pub url: String,
    pub id: Option<String>,
    pub icon: Option<String>,
}

/// The Softwell Ed25519 public key (base64) built into this binary.
pub fn public_key() -> Result<&'static str, String> {
    option_env!("SOFTWELL_LINK_PUBLIC_KEY").ok_or_else(|| "Questa build non contiene la chiave pubblica SoftWell.".into())
}

pub fn verify(text: &str) -> Result<Connection, String> {
    verify_with(text, public_key()?)
}

/// The payload of a `{payload, signature}` envelope, after checking its Ed25519 signature with `key` (base64).
pub fn signed_payload(text: &str, key: &str) -> Result<Vec<u8>, String> {
    let envelope: Envelope =
        serde_json::from_str(text).map_err(|_| "Formato del documento firmato non valido.")?;
    let payload = STANDARD
        .decode(envelope.payload)
        .map_err(|_| "Contenuto non valido.")?;
    let signature = STANDARD
        .decode(envelope.signature)
        .map_err(|_| "Firma non valida.")?;
    let public_key = STANDARD
        .decode(key)
        .map_err(|_| "Chiave del runner non valida.")?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(&payload, &signature)
        .map_err(|_| "Firma SoftWell non valida.")?;
    Ok(payload)
}

fn verify_with(text: &str, key: &str) -> Result<Connection, String> {
    if text.len() > MAX_SIZE {
        return Err("Collegamento troppo grande.".into());
    }
    let payload = signed_payload(text, key)?;
    let link: Connection =
        serde_json::from_slice(&payload).map_err(|_| "Contenuto del collegamento non valido.")?;
    let url = Url::parse(&link.url).map_err(|_| "Indirizzo non valido.")?;
    if link.format != "it.softwell.runner.link/v1"
        || link.name.trim().is_empty()
        || link.name.chars().any(char::is_control)
        || url.scheme() != "https"
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(
            "Il collegamento deve indicare un gestionale e un indirizzo HTTPS valido.".into(),
        );
    }
    if let Some(icon) = &link.icon {
        let data = STANDARD.decode(icon).map_err(|_| "Icona non valida.")?;
        if data.len() < 24 || !data.starts_with(b"\x89PNG\r\n\x1a\n") ||
            data[16..24] != [0, 0, 0, 32, 0, 0, 0, 32] ||
            tauri::image::Image::from_bytes(&data).is_err() {
            return Err("Usa una icona PNG 32×32.".into());
        }
    }
    Ok(link)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::{
        rand::SystemRandom,
        signature::{Ed25519KeyPair, KeyPair},
    };

    #[test]
    fn only_authentic_https_links_are_accepted() {
        let keys = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let keys = Ed25519KeyPair::from_pkcs8(keys.as_ref()).unwrap();
        let key = STANDARD.encode(keys.public_key());
        let sign = |url: &str| {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "name":"Studio","url":url,"format":"it.softwell.runner.link/v1"
            }))
            .unwrap();
            serde_json::json!({"payload":STANDARD.encode(&bytes),"signature":STANDARD.encode(keys.sign(&bytes))})
        };
        let mut good = sign("https://example.com/app");
        assert_eq!(verify_with(&good.to_string(), &key).unwrap().name, "Studio");
        for url in ["http://example.com", "https://name:password@example.com"] {
            assert!(verify_with(&sign(url).to_string(), &key).is_err());
        }
        good["payload"] = sign("https://other.example")["payload"].clone();
        assert!(verify_with(&good.to_string(), &key).is_err());
        assert!(verify_with("{}", &key).is_err());
    }
}
