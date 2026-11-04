//! Signing cards read through the bit4id PKCS#11 library.
//!
//! Port of `firmatore_mac/tessere.py` (icond, branch firma-digitale-protocolli).
//! A card is identified by the sha256 of its signing certificate (DER), never by the
//! reader: the reader order changes between launches. The PIN is only used for the
//! login and is not kept; the logged-in session stays open until it expires, the card
//! is removed or the app exits.

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use cryptoki::context::{CInitializeArgs, CInitializeFlags, Pkcs11};
use cryptoki::mechanism::Mechanism;
use cryptoki::object::{Attribute, AttributeType, ObjectClass};
use cryptoki::session::{Session, UserType};
use cryptoki::slot::Slot;
use cryptoki::types::AuthPin;
use serde::Serialize;
use sha2::{Digest, Sha256};
use x509_parser::prelude::{FromDer, X509Certificate};

/// DER prefix of a SHA-256 DigestInfo: the card signs prefix + 32-byte digest (RSA PKCS#1 v1.5).
const DIGESTINFO_SHA256: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04,
    0x20,
];

/// Default library locations, as in `sistema.py`. On Windows Firma4NG installs the
/// library only at 32 bit, so the app must be a 32-bit build there.
#[cfg(target_os = "macos")]
const LIBRARIES: &[&str] = &["/Library/bit4id/pkcs11/libbit4xpki.dylib"];
#[cfg(target_os = "windows")]
const LIBRARIES: &[&str] = &[
    r"C:\Program Files (x86)\Bit4id\UKC\UKC\etc\idprovider_smartcard\bit4xpki.dll",
    r"C:\Program Files (x86)\Bit4id\Firma4ng-InfoCamere\Main\bit4xpki.dll",
    r"C:\Windows\System32\bit4xpki.dll",
];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const LIBRARIES: &[&str] = &["/usr/lib/libbit4xpki.so"];

/// The configured library, or the first default one that exists.
pub fn library_path(configured: Option<&str>) -> Result<String, String> {
    if let Some(path) = configured {
        return Ok(path.to_string());
    }
    LIBRARIES
        .iter()
        .find(|path| Path::new(path).exists())
        .map(|path| path.to_string())
        .ok_or_else(|| format!("PKCS#11 library not found in: {}", LIBRARIES.join(", ")))
}

struct Card {
    fingerprint: String,
    holder: String,
    serial: String,
    valid_until: String,
    der: Vec<u8>,
    ckid: Vec<u8>,
    slot: Slot,
    session: Option<Session>,
    open_until: Option<SystemTime>,
}

/// What the page receives for each card in the readers.
#[derive(Serialize)]
pub struct CardInfo {
    fingerprint: String,
    holder: String,
    serial: String,
    valid_until: String,
    /// DER, base64.
    certificate: String,
    open: bool,
}

/// What the PIN window shows.
#[derive(Serialize)]
pub struct PinRequest {
    holder: String,
    min_length: usize,
    max_length: usize,
    /// At least one wrong PIN since the last right one: the user can still retry.
    wrong_pin_before: bool,
}

pub struct Cards {
    pkcs11: Pkcs11,
    cards: HashMap<String, Card>,
}

impl Cards {
    pub fn load(library: &str) -> Result<Self, String> {
        let pkcs11 = Pkcs11::new(library).map_err(|e| format!("{library}: {e}"))?;
        pkcs11
            .initialize(CInitializeArgs::new(CInitializeFlags::OS_LOCKING_OK))
            .map_err(|e| format!("{library}: {e}"))?;
        Ok(Self { pkcs11, cards: HashMap::new() })
    }

    /// Re-reads the readers without PIN. New cards enter closed, removed cards leave
    /// (their session is dropped), present cards keep their session.
    pub fn refresh(&mut self) -> Result<(), String> {
        let mut seen = HashMap::new();
        for slot in self.pkcs11.get_slots_with_token().map_err(|e| e.to_string())? {
            // A card the library cannot read is skipped, as in tessere.py.
            if let Ok(Some(card)) = self.read(slot) {
                seen.insert(card.fingerprint.clone(), card);
            }
        }
        self.cards.retain(|fingerprint, _| seen.contains_key(fingerprint));
        for (fingerprint, card) in seen {
            match self.cards.get_mut(&fingerprint) {
                Some(known) => known.slot = card.slot,
                None => {
                    self.cards.insert(fingerprint, card);
                }
            }
        }
        Ok(())
    }

    pub fn list(&mut self) -> Vec<CardInfo> {
        let fingerprints: Vec<String> = self.cards.keys().cloned().collect();
        fingerprints
            .into_iter()
            .map(|fingerprint| {
                let open = self.is_open(&fingerprint);
                let card = &self.cards[&fingerprint];
                CardInfo {
                    fingerprint,
                    holder: card.holder.clone(),
                    serial: card.serial.clone(),
                    valid_until: card.valid_until.clone(),
                    certificate: STANDARD.encode(&card.der),
                    open,
                }
            })
            .collect()
    }

    /// Whether the card has a logged-in session that has not expired. An expired one is closed.
    pub fn is_open(&mut self, fingerprint: &str) -> bool {
        let Some(card) = self.cards.get_mut(fingerprint) else {
            return false;
        };
        if card.session.is_some() && card.open_until.is_some_and(|until| SystemTime::now() >= until) {
            close_card(card);
        }
        card.session.is_some()
    }

    /// Refuses when the card signals its last PIN attempt or a locked PIN (PUK needed).
    pub fn pin_request(&self, fingerprint: &str) -> Result<PinRequest, String> {
        let card = self.card(fingerprint)?;
        let info = self.pkcs11.get_token_info(card.slot).map_err(|e| e.to_string())?;
        if info.user_pin_final_try() || info.user_pin_locked() {
            return Err(format!(
                "the card signals {}: the PIN is not tried",
                if info.user_pin_locked() { "a locked PIN" } else { "the last PIN attempt" }
            ));
        }
        Ok(PinRequest {
            holder: card.holder.clone(),
            min_length: info.min_pin_length(),
            max_length: info.max_pin_length(),
            wrong_pin_before: info.user_pin_count_low(),
        })
    }

    /// Login with the PIN, one attempt. A PIN of the wrong length is not tried (it would cost an attempt).
    pub fn open(&mut self, fingerprint: &str, pin: &str, open_until: SystemTime) -> Result<(), String> {
        let request = self.pin_request(fingerprint)?;
        let length = pin.chars().count();
        if length < request.min_length || length > request.max_length {
            return Err(format!(
                "the PIN must have {} to {} characters: not tried",
                request.min_length, request.max_length
            ));
        }
        let card = self.cards.get_mut(fingerprint).unwrap();
        close_card(card);
        let session = self.pkcs11.open_ro_session(card.slot).map_err(|e| e.to_string())?;
        session
            .login(UserType::User, Some(&AuthPin::from(pin)))
            .map_err(|e| format!("PIN not accepted ({e})"))?;
        card.session = Some(session);
        card.open_until = Some(open_until);
        Ok(())
    }

    /// RSA PKCS#1 v1.5 signature of the SHA-256 DigestInfo of `digest` (32 bytes), with the card open.
    /// If the card no longer answers, its session is closed and no new PIN is asked from here.
    pub fn sign(&mut self, fingerprint: &str, digest: &[u8]) -> Result<Vec<u8>, String> {
        if digest.len() != 32 {
            return Err(format!("the digest must be 32 bytes, got {}", digest.len()));
        }
        if !self.is_open(fingerprint) {
            return Err("card closed".to_string());
        }
        let card = self.cards.get_mut(fingerprint).unwrap();
        let session = card.session.as_ref().unwrap();
        let template = [Attribute::Class(ObjectClass::PRIVATE_KEY), Attribute::Id(card.ckid.clone())];
        let data = [&DIGESTINFO_SHA256[..], digest].concat();
        let result = session
            .find_objects(&template)
            .map_err(|e| e.to_string())
            .and_then(|keys| keys.first().copied().ok_or_else(|| "private key not found".to_string()))
            .and_then(|key| session.sign(&Mechanism::RsaPkcs, key, &data).map_err(|e| e.to_string()));
        if let Err(e) = &result {
            close_card(card);
            return Err(format!("the card does not answer ({e}): closed"));
        }
        result
    }

    pub fn close(&mut self, fingerprint: &str) -> Result<(), String> {
        close_card(self.cards.get_mut(fingerprint).ok_or_else(|| unknown(fingerprint))?);
        Ok(())
    }

    fn card(&self, fingerprint: &str) -> Result<&Card, String> {
        self.cards.get(fingerprint).ok_or_else(|| unknown(fingerprint))
    }

    /// The signing certificate (keyUsage nonRepudiation) of the card in `slot`, without PIN.
    fn read(&self, slot: Slot) -> Result<Option<Card>, String> {
        let session = self.pkcs11.open_ro_session(slot).map_err(|e| e.to_string())?;
        let objects = session
            .find_objects(&[Attribute::Class(ObjectClass::CERTIFICATE)])
            .map_err(|e| e.to_string())?;
        for object in objects {
            let attributes = session
                .get_attributes(object, &[AttributeType::Value, AttributeType::Id])
                .map_err(|e| e.to_string())?;
            let (mut der, mut ckid) = (Vec::new(), Vec::new());
            for attribute in attributes {
                match attribute {
                    Attribute::Value(value) => der = value,
                    Attribute::Id(id) => ckid = id,
                    _ => {}
                }
            }
            if let Some(card) = signing_card(slot, der, ckid)? {
                return Ok(Some(card));
            }
        }
        Ok(None)
    }
}

fn signing_card(slot: Slot, der: Vec<u8>, ckid: Vec<u8>) -> Result<Option<Card>, String> {
    let (_, certificate) = X509Certificate::from_der(&der).map_err(|e| e.to_string())?;
    let non_repudiation = certificate
        .key_usage()
        .map_err(|e| e.to_string())?
        .is_some_and(|usage| usage.value.non_repudiation());
    if !non_repudiation {
        return Ok(None);
    }
    let common_name = certificate
        .subject()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .unwrap_or("?");
    Ok(Some(Card {
        fingerprint: hex(&Sha256::digest(&der)),
        holder: title_case(common_name),
        serial: format!("{:X}", certificate.serial),
        valid_until: certificate.validity().not_after.to_datetime().date().to_string(),
        der,
        ckid,
        slot,
        session: None,
        open_until: None,
    }))
}

/// Logout and close; a card already removed does not answer, and that is fine.
fn close_card(card: &mut Card) {
    card.open_until = None;
    if let Some(session) = card.session.take() {
        let _ = session.logout();
        let _ = session.close();
    }
}

fn unknown(fingerprint: &str) -> String {
    format!("no card with fingerprint {fingerprint} in the readers")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Python `str.title()` on the whitespace-normalised common name, as tessere.py does.
fn title_case(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}
