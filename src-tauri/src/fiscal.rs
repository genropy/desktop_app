//! Fiscal cash registers on the local network, for the page of a saved site.
//!
//! The page calls `fiscal_status`, `fiscal_receipt`, `fiscal_x_report` and `fiscal_daily_closure`. They are granted at
//! runtime only to the windows of saved sites, for the origin of the site; each one also checks that `origin` is the
//! origin of the calling page and of a saved site. The address of the register is configured in the window Gestione
//! (`manage_fiscal_*`), saved in `<app config dir>/fiscal.json` keyed by the origin of the site, never by the page.
//!
//! The only register today is Epson FP-81 II RT / FP-90 III RT (`epson.rs`). See `xmldocs/proposal/fiscal-epson.xml`.
mod epson;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::ipc::CapabilityBuilder;
use tauri::{AppHandle, Manager, Url, Webview};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::sites;

/// HTTP timeout of a status, receipt or X report: the `timeout` of the URL is 10 s (`epson::PATH`).
const TIMEOUT: Duration = Duration::from_secs(20);
/// HTTP timeout of the Z report: longer than its `timeout` of 30 s (`epson::Z_TIMEOUT_MS`).
const CLOSURE_TIMEOUT: Duration = Duration::from_secs(60);
const NOT_CONFIGURED: &str = "Nessun registratore di cassa configurato per questo sito: aggiungilo in Gestione.";

/// The register of one site, as saved in `fiscal.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Register {
    /// Host name or IP address, with an optional port.
    pub host: String,
    /// The register has SSL MODE 1 or 2 (SET 34): only HTTPS on port 443 answers.
    #[serde(default)]
    pub https: bool,
    /// SHA-256 of the certificate the user confirmed in Gestione, uppercase hex pairs separated by `:`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

impl Register {
    fn base_url(&self) -> Result<Url, String> {
        let scheme = if self.https { "https" } else { "http" };
        let url = Url::parse(&format!("{scheme}://{}/", self.host.trim()))
            .map_err(|_| format!("Indirizzo del registratore non valido: {}", self.host))?;
        if url.host_str().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(format!("Indirizzo del registratore non valido: {}", self.host));
        }
        Ok(url)
    }
}

/// Window labels that received the fiscal commands, and one lock per register.
#[derive(Default)]
pub struct Registers {
    grants: Mutex<HashSet<String>>,
    /// fpmate resets the register at every request (Guide §3.13): one request at a time per register.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// Grants the fiscal commands to the pages of `url`'s origin in the window `label`.
pub fn grant(app: &AppHandle, url: &Url, label: &str) -> tauri::Result<()> {
    let registers = app.state::<Registers>();
    let mut labels = registers.grants.lock().unwrap();
    if labels.contains(label) {
        return Ok(());
    }
    let origin = url.origin().ascii_serialization();
    let capability = CapabilityBuilder::new(format!("fiscal-{}", labels.len()))
        .remote(format!("{origin}/*"))
        .local(false)
        .window(label)
        .permission("allow-fiscal-status")
        .permission("allow-fiscal-receipt")
        .permission("allow-fiscal-x-report")
        .permission("allow-fiscal-daily-closure");
    app.add_capability(capability)?;
    labels.insert(label.into());
    Ok(())
}

fn path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|e| e.to_string())?.join("fiscal.json"))
}

fn load(app: &AppHandle) -> Result<BTreeMap<String, Register>, String> {
    let path = path(app)?;
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn save(app: &AppHandle, registers: &BTreeMap<String, Register>) -> Result<(), String> {
    let path = path(app)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(registers).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Removes the register of `origin`, when the site is removed.
pub fn forget(app: &AppHandle, origin: &str) -> Result<(), String> {
    let mut registers = load(app)?;
    if registers.remove(origin).is_some() {
        save(app, &registers)?;
    }
    Ok(())
}

fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// Accepts the certificate whose SHA-256 is `expected`, and records the one it saw. With `expected` `None`
/// (only the test in Gestione) it accepts any certificate, so the user can read and confirm its fingerprint.
/// The handshake signatures are verified in both cases.
#[derive(Debug)]
struct Pinned {
    expected: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let seen = fingerprint(end_entity.as_ref());
        *self.seen.lock().unwrap() = Some(seen.clone());
        match &self.expected {
            Some(expected) if *expected != seen => Err(rustls::Error::General(
                "il certificato del registratore non è quello confermato in Gestione".into(),
            )),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// What a request returns: the body of the answer and, over HTTPS, the fingerprint of the certificate.
struct Exchange {
    body: String,
    fingerprint: Option<String>,
}

/// Posts `xml` to the register, after the previous request to the same register has finished.
async fn send(
    app: &AppHandle,
    register: &Register,
    expected: Option<String>,
    xml: String,
    timeout: Duration,
) -> Result<Exchange, String> {
    let base = register.base_url()?;
    let lock = {
        let registers = app.state::<Registers>();
        let mut locks = registers.locks.lock().unwrap();
        locks.entry(base.origin().ascii_serialization()).or_default().clone()
    };
    let _turn = lock.lock().await;
    let seen = Arc::new(Mutex::new(None));
    let mut builder = reqwest::Client::builder().timeout(timeout).redirect(reqwest::redirect::Policy::none());
    if register.https {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Pinned { expected, seen: seen.clone(), provider: provider.clone() };
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        builder = builder.tls_backend_preconfigured(config);
    }
    let client = builder.build().map_err(|e| e.to_string())?;
    let address = base.join(epson::PATH).map_err(|e| e.to_string())?;
    let host = &register.host;
    let response = client
        .post(address)
        .header("Content-Type", "text/xml; charset=utf-8")
        .header("If-Modified-Since", "Thu, 01 Jan 1970 00:00:00 GMT")
        .header("SOAPAction", "\"\"")
        .body(xml)
        .send()
        .await
        .map_err(|e| format!("Impossibile raggiungere il registratore {host}: {e}"))?;
    let code = response.status();
    if code == reqwest::StatusCode::UNAUTHORIZED {
        return Err(format!("Il registratore {host} chiede utente e password: GenroApp non li gestisce ancora."));
    }
    if !code.is_success() {
        return Err(format!("Il registratore {host} risponde HTTP {code}."));
    }
    let body = response.text().await.map_err(|e| e.to_string())?;
    let fingerprint = seen.lock().unwrap().clone();
    Ok(Exchange { body, fingerprint })
}

/// The register of the site `origin` and the fingerprint its certificate must have.
fn configured(app: &AppHandle, origin: &str) -> Result<Option<(Register, Option<String>)>, String> {
    let Some(register) = load(app)?.remove(origin) else {
        return Ok(None);
    };
    if register.https && register.fingerprint.is_none() {
        return Err(format!("Il certificato del registratore {} non è stato confermato in Gestione.", register.host));
    }
    let expected = register.fingerprint.clone();
    Ok(Some((register, expected)))
}

fn required(app: &AppHandle, origin: &str) -> Result<(Register, Option<String>), String> {
    configured(app, origin)?.ok_or_else(|| NOT_CONFIGURED.to_string())
}

async fn status_of(app: &AppHandle, register: &Register, expected: Option<String>) -> Result<(epson::Status, Option<String>), String> {
    let basic = send(app, register, expected.clone(), epson::status_request(false), TIMEOUT).await?;
    let rt = send(app, register, expected, epson::status_request(true), TIMEOUT).await?;
    let status = epson::status(epson::parse_answer(&basic.body)?, epson::parse_answer(&rt.body)?)?;
    Ok((status, basic.fingerprint))
}

#[derive(Serialize)]
pub struct SiteStatus {
    configured: bool,
    status: Option<epson::Status>,
}

#[tauri::command]
pub async fn fiscal_status(app: AppHandle, webview: Webview, origin: String) -> Result<SiteStatus, String> {
    crate::device_login::caller_site(&app, &webview, &origin)?;
    let Some((register, expected)) = configured(&app, &origin)? else {
        return Ok(SiteStatus { configured: false, status: None });
    };
    let (status, _) = status_of(&app, &register, expected).await?;
    Ok(SiteStatus { configured: true, status: Some(status) })
}

#[tauri::command]
pub async fn fiscal_receipt(
    app: AppHandle,
    webview: Webview,
    origin: String,
    lines: Vec<epson::Line>,
    payments: Vec<epson::Payment>,
) -> Result<epson::Receipt, String> {
    crate::device_login::caller_site(&app, &webview, &origin)?;
    let xml = epson::receipt_request(&lines, &payments)?;
    let (register, expected) = required(&app, &origin)?;
    let answer = send(&app, &register, expected, xml, TIMEOUT).await?;
    epson::receipt(epson::parse_answer(&answer.body)?)
}

#[tauri::command]
pub async fn fiscal_x_report(app: AppHandle, webview: Webview, origin: String) -> Result<epson::Report, String> {
    crate::device_login::caller_site(&app, &webview, &origin)?;
    let (register, expected) = required(&app, &origin)?;
    let answer = send(&app, &register, expected, epson::report_request(false), TIMEOUT).await?;
    epson::report(epson::parse_answer(&answer.body)?)
}

/// The daily closure (Z report). GenroApp asks the user first: the closure cannot be undone.
#[tauri::command]
pub async fn fiscal_daily_closure(app: AppHandle, webview: Webview, origin: String) -> Result<epson::Report, String> {
    let site = crate::device_login::caller_site(&app, &webview, &origin)?;
    let (register, expected) = required(&app, &origin)?;
    let question = format!(
        "Eseguire la chiusura giornaliera sul registratore {} per «{}»?\n\nLa chiusura non si può annullare.",
        register.host, site.name
    );
    let dialog = app.clone();
    let confirmed = tauri::async_runtime::spawn_blocking(move || {
        dialog
            .dialog()
            .message(question)
            .title("Chiusura giornaliera")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom("Esegui la chiusura".into(), "Annulla".into()))
            .blocking_show()
    })
    .await
    .map_err(|e| e.to_string())?;
    if !confirmed {
        return Err("Chiusura giornaliera annullata dall'utente.".into());
    }
    let answer = send(&app, &register, expected, epson::report_request(true), CLOSURE_TIMEOUT).await?;
    epson::report(epson::parse_answer(&answer.body)?)
}

// ------------------------------------------------------------------ window Gestione

#[tauri::command]
pub fn manage_fiscal_get(app: AppHandle, origin: String) -> Result<Option<Register>, String> {
    Ok(load(&app)?.remove(&origin))
}

/// Saves the register of the site `origin`; an empty `host` removes it. Over HTTPS `fingerprint` is the one the
/// user saw in the test and confirmed.
#[tauri::command]
pub fn manage_fiscal_set(
    app: AppHandle,
    origin: String,
    host: String,
    https: bool,
    fingerprint: Option<String>,
) -> Result<(), String> {
    sites::find(&app, &origin)?.ok_or("Sito non trovato.")?;
    let mut registers = load(&app)?;
    if host.trim().is_empty() {
        registers.remove(&origin);
        return save(&app, &registers);
    }
    let fingerprint = if https {
        Some(fingerprint.ok_or("Prova il registratore e conferma il suo certificato prima di salvare.")?)
    } else {
        None
    };
    let register = Register { host: host.trim().to_string(), https, fingerprint };
    register.base_url()?;
    registers.insert(origin, register);
    save(&app, &registers)
}

#[derive(Serialize)]
pub struct Test {
    status: epson::Status,
    /// Over HTTPS: the fingerprint of the certificate the register presented.
    fingerprint: Option<String>,
}

/// Asks the status of the register at `host`, accepting any certificate, and returns its fingerprint.
#[tauri::command]
pub async fn manage_fiscal_test(app: AppHandle, host: String, https: bool) -> Result<Test, String> {
    let register = Register { host: host.trim().to_string(), https, fingerprint: None };
    let (status, fingerprint) = status_of(&app, &register, None).await?;
    Ok(Test { status, fingerprint })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register(host: &str, https: bool) -> Register {
        Register { host: host.into(), https, fingerprint: None }
    }

    #[test]
    fn base_url_of_a_register() {
        assert_eq!(register("192.168.1.20", false).base_url().unwrap().as_str(), "http://192.168.1.20/");
        assert_eq!(register(" cassa.local:8080 ", false).base_url().unwrap().as_str(), "http://cassa.local:8080/");
        assert_eq!(register("192.168.1.20", true).base_url().unwrap().as_str(), "https://192.168.1.20/");
        for bad in ["", "192.168.1.20/cgi-bin", "user@192.168.1.20", "192.168.1.20?x=1", "a b"] {
            assert!(register(bad, false).base_url().is_err(), "{bad}");
        }
    }

    #[test]
    fn request_address() {
        let url = register("10.0.0.5", false).base_url().unwrap().join(epson::PATH).unwrap();
        assert_eq!(
            url.as_str(),
            "http://10.0.0.5/cgi-bin/fpmate.cgi?devid=local_printer&timeout=10000&reply_decimal_separator=."
        );
    }

    #[test]
    fn fingerprint_format() {
        let text = fingerprint(b"abc");
        assert_eq!(text.len(), 32 * 3 - 1);
        assert!(text.starts_with("BA:78:16:BF:"));
    }

    #[test]
    fn register_file_format() {
        let saved = Register { host: "10.0.0.5".into(), https: true, fingerprint: Some("AA:BB".into()) };
        let text = serde_json::to_string(&saved).unwrap();
        assert_eq!(text, r#"{"host":"10.0.0.5","https":true,"fingerprint":"AA:BB"}"#);
        assert_eq!(serde_json::from_str::<Register>(r#"{"host":"10.0.0.5"}"#).unwrap(), register("10.0.0.5", false));
    }
}
