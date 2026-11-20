//! macOS: a P-256 key generated in the Secure Enclave and kept in the data protection keychain of the account.
//!
//! The private key never leaves the Secure Enclave. Each signature asks Touch ID, or also the password of the Mac
//! when the key was created on a Mac without Touch ID. The keychain needs the entitlement `keychain-access-groups`
//! (`GenroApp.entitlements`): an unsigned build gets the error -34018.
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::error::CFError;
use objc2::rc::Retained;
use core_foundation::string::{CFString, CFStringRef};
use objc2_foundation::NSString;
use objc2_local_authentication::{LAContext, LAPolicy};
use security_framework::access_control::{ProtectionMode, SecAccessControl};
use security_framework::item::{ItemClass, ItemSearchOptions, KeyClass, Location, Reference, SearchResult};
use security_framework::key::{Algorithm, GenerateKeyOptions, KeyType, SecKey, Token};

use super::Verification;

pub const PLATFORM: &str = "macos";

// SecAccessControlCreateFlags
const USER_PRESENCE: usize = 1 << 0;
const BIOMETRY_CURRENT_SET: usize = 1 << 3;
const PRIVATE_KEY_USAGE: usize = 1 << 30;

const ERR_ITEM_NOT_FOUND: i32 = -25300;
const ERR_MISSING_ENTITLEMENT: i64 = -34018;
const ERR_USER_CANCELED: i64 = -128;
const LA_ERROR_USER_CANCEL: i64 = -2;

fn has_biometry() -> bool {
    let context = unsafe { LAContext::new() };
    unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics) }.is_ok()
}

fn cf_message(error: &CFError) -> String {
    match error.code() as i64 {
        ERR_MISSING_ENTITLEMENT => "GenroApp non è firmato con l'accesso al portachiavi: la chiave non può essere creata.".into(),
        ERR_USER_CANCELED | LA_ERROR_USER_CANCEL => "Verifica annullata.".into(),
        _ => error.description().to_string(),
    }
}

/// The private key of `label`. With `context`, its first use asks the user with the reason of the context.
fn find(label: &str, context: Option<&Retained<LAContext>>) -> Result<Option<SecKey>, String> {
    let mut search = ItemSearchOptions::new();
    search
        .class(ItemClass::key())
        .key_class(KeyClass::private())
        .label(label)
        .ignore_legacy_keychains()
        .load_refs(true);
    if let Some(context) = context {
        // An LAContext is an Objective-C object: CoreFoundation retains and releases it like a CFType.
        let context = unsafe { CFType::wrap_under_get_rule(Retained::as_ptr(context) as CFTypeRef) };
        search.local_authentication_context(Some(context));
    }
    match search.search() {
        Ok(results) => Ok(results.into_iter().find_map(|result| match result {
            SearchResult::Ref(Reference::Key(key)) => Some(key),
            _ => None,
        })),
        Err(e) if e.code() == ERR_ITEM_NOT_FOUND => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn exists(label: &str) -> bool {
    matches!(find(label, None), Ok(Some(_)))
}

/// The public key of `label` as an X9.63 uncompressed point. Reading it asks nothing to the user.
pub fn public_point(label: &str) -> Result<Vec<u8>, String> {
    let key = find(label, None)?.ok_or("Questo computer non ha una chiave per il sito.")?;
    let public = key
        .public_key()
        .and_then(|public| public.external_representation())
        .ok_or("La chiave pubblica non è leggibile.")?;
    Ok(public.to_vec())
}

/// Creates the key of `label`: finger only on a Mac with Touch ID, finger or password otherwise.
/// Returns the public key as an X9.63 uncompressed point.
pub fn create(label: &str) -> Result<(Vec<u8>, Verification), String> {
    let verification = if has_biometry() { Verification::Biometry } else { Verification::UserPresence };
    let presence = match verification {
        Verification::Biometry => BIOMETRY_CURRENT_SET,
        Verification::UserPresence => USER_PRESENCE,
    };
    let access = SecAccessControl::create_with_protection(
        Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
        (PRIVATE_KEY_USAGE | presence) as _,
    )
    .map_err(|e| e.to_string())?;
    let mut options = GenerateKeyOptions::default();
    options
        .set_key_type(KeyType::ec_sec_prime_random())
        .set_size_in_bits(256)
        .set_label(label)
        .set_token(Token::SecureEnclave)
        .set_location(Location::DataProtectionKeychain)
        .set_access_control(access);
    let key = SecKey::new(&options).map_err(|e| cf_message(&e))?;
    let public = key
        .public_key()
        .and_then(|public| public.external_representation())
        .ok_or("La chiave pubblica non è leggibile.")?;
    Ok((public.to_vec(), verification))
}

/// Signs `message` with the key of `label` after Touch ID (or the password, per the key), showing `reason`.
pub fn sign(label: &str, reason: &str, message: &[u8]) -> Result<Vec<u8>, String> {
    let context = unsafe { LAContext::new() };
    unsafe { context.setLocalizedReason(&NSString::from_str(reason)) };
    let key = find(label, Some(&context))?.ok_or("Questo computer non ha una chiave per il sito.")?;
    key.create_signature(Algorithm::ECDSASignatureMessageX962SHA256, message)
        .map_err(|e| cf_message(&e))
}

pub fn delete(label: &str) -> Result<(), String> {
    let mut search = ItemSearchOptions::new();
    search.class(ItemClass::key()).label(label).ignore_legacy_keychains();
    match search.delete() {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ERR_ITEM_NOT_FOUND => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCopyComputerName(store: *const std::ffi::c_void, encoding: *mut u32) -> CFStringRef;
}

/// The name of the Mac shown in the sharing settings, for the device list of the site.
pub fn device_name() -> String {
    let name = unsafe { SCDynamicStoreCopyComputerName(std::ptr::null(), std::ptr::null_mut()) };
    if name.is_null() {
        return String::new();
    }
    unsafe { CFString::wrap_under_create_rule(name) }.to_string()
}
