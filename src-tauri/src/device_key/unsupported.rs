//! Platforms without a device key yet. Windows Hello (`KeyCredentialManager`) comes here next.
use super::Verification;

#[cfg(target_os = "windows")]
pub const PLATFORM: &str = "windows";
#[cfg(not(target_os = "windows"))]
pub const PLATFORM: &str = "unsupported";

const MISSING: &str = "L'accesso con questo computer non è ancora disponibile su questo sistema.";

pub fn exists(_label: &str) -> bool {
    false
}

/// The public key (X9.63 uncompressed point) and the verification of a new key.
pub fn create(_label: &str) -> Result<(Vec<u8>, Verification), String> {
    Err(MISSING.into())
}

/// ECDSA P-256 SHA-256 signature (DER) of `message`, after the user proves presence.
pub fn sign(_label: &str, _reason: &str, _message: &[u8]) -> Result<Vec<u8>, String> {
    Err(MISSING.into())
}

pub fn delete(_label: &str) -> Result<(), String> {
    Ok(())
}

pub fn device_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}
