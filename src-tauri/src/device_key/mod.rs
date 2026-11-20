//! The key of this computer for one site, kept in secure hardware.
//!
//! Every platform gives the same functions: `exists`, `create`, `sign`, `delete`, `device_name` and `PLATFORM`.
//! macOS keeps a P-256 key in the Secure Enclave; Windows (TPM through Windows Hello) is not implemented yet.
//! A key is named by the origin of its site.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(not(target_os = "macos"))]
pub use unsupported::*;

/// How the user proves presence before each signature. Fixed when the key is created.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verification {
    /// Only an enrolled finger, invalidated when the fingers change.
    Biometry,
    /// A finger or the password of the computer.
    UserPresence,
}

impl Verification {
    pub fn as_str(self) -> &'static str {
        match self {
            Verification::Biometry => "biometry",
            Verification::UserPresence => "user_presence",
        }
    }
}

/// The name of the key of `origin` in the store of the platform.
pub fn label(origin: &str) -> String {
    format!("GenroApp device {origin}")
}
