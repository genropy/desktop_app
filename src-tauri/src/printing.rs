//! Local printing for the pages of saved sites, with no print dialog: PDF through CUPS `lp`, raw ZPL to label
//! printers.
//!
//! The commands are granted at runtime only to the windows of saved sites, for the origin of the site.
//!
//! Label printers are named in `config.json`, key `label_printers`: name → `tcp://<host>[:<port>]` (raw ZPL on
//! TCP, port 9100 when omitted) or the name of a raw CUPS queue (a USB printer, `lp -o raw`). The page passes only
//! the name, so a remote page never opens a connection to a host it chooses.
use std::collections::HashSet;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Serialize;
use tauri::ipc::CapabilityBuilder;
use tauri::{AppHandle, Manager, Url};

use crate::load_config;

const ZPL_PORT: u16 = 9100;
const ZPL_TIMEOUT: Duration = Duration::from_secs(10);

/// Window labels that already received the print commands.
#[derive(Default)]
pub struct Grants(Mutex<HashSet<String>>);

/// Grants the print commands to the pages of `url`'s origin in the window `label`.
pub fn grant(app: &AppHandle, url: &Url, label: &str) -> tauri::Result<()> {
    let grants = app.state::<Grants>();
    let mut labels = grants.0.lock().unwrap();
    if labels.contains(label) {
        return Ok(());
    }
    let origin = url.origin().ascii_serialization();
    let capability = CapabilityBuilder::new(format!("printing-{}", labels.len()))
        .remote(format!("{origin}/*"))
        .local(false)
        .window(label)
        .permission("allow-printer-list")
        .permission("allow-print-pdf")
        .permission("allow-print-zpl");
    app.add_capability(capability)?;
    labels.insert(label.into());
    Ok(())
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A CUPS printer: `print_pdf`.
    Cups,
    /// A label printer of `config.json`: `print_zpl`.
    Label,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Printer {
    name: String,
    kind: Kind,
    /// The CUPS default destination.
    default: bool,
}

/// Where `print_zpl` sends the ZPL of a label printer.
#[derive(Debug, PartialEq)]
enum LabelTarget {
    Tcp(String, u16),
    Queue(String),
}

fn label_target(address: &str) -> Result<LabelTarget, String> {
    let Some(rest) = address.strip_prefix("tcp://") else {
        return Ok(LabelTarget::Queue(address.to_string()));
    };
    let rest = rest.trim_end_matches('/');
    let invalid = || format!("Indirizzo non valido: {address}.");
    // An IPv6 host is written in brackets, so its colons are not the port separator.
    let (host, port) = match rest.strip_prefix('[') {
        Some(bracketed) => {
            let (host, after) = bracketed.split_once(']').ok_or_else(invalid)?;
            (host, after.strip_prefix(':'))
        }
        None => match rest.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (rest, None),
        },
    };
    if host.is_empty() {
        return Err(invalid());
    }
    let port = match port {
        Some(port) => port.parse().map_err(|_| format!("Porta non valida in {address}."))?,
        None => ZPL_PORT,
    };
    Ok(LabelTarget::Tcp(host.to_string(), port))
}

/// Runs a CUPS command and returns its stdout. The messages of CUPS on macOS follow the language of the system,
/// also with `LC_ALL=C`: the parsers below never read the words.
fn cups(program: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Impossibile eseguire {program} (CUPS): {e}"))?;
    if let Some(data) = stdin {
        child.stdin.take().unwrap().write_all(data).map_err(|e| format!("{program}: {e}"))?;
    }
    let output = child.wait_with_output().map_err(|e| format!("{program}: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{program}: {}", stderr.trim()));
    }
    Ok(stdout)
}

/// The destinations of `lpstat -e`, one per line.
fn parse_destinations(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|line| !line.is_empty()).map(String::from).collect()
}

/// The default destination of `lpstat -d` ("<message>: <name>"), when it is one of `destinations`.
fn parse_default(text: &str, destinations: &[String]) -> Option<String> {
    let name = text.trim().split_once(':')?.1.trim();
    destinations.iter().find(|destination| *destination == name).cloned()
}

/// The request id `<printer>-<number>` in the answer of `lp`, or the whole answer when it has none.
fn parse_request(text: &str, printer: &str) -> String {
    let text = text.trim();
    text.split_whitespace()
        .find(|word| {
            word.strip_prefix(printer)
                .and_then(|rest| rest.strip_prefix('-'))
                .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
        })
        .unwrap_or(text)
        .to_string()
}

fn cups_printers() -> Result<Vec<String>, String> {
    Ok(parse_destinations(&cups("lpstat", &["-e"], None)?))
}

fn send_lp(printer: &str, options: &[&str], data: &[u8]) -> Result<String, String> {
    let mut args = vec!["-d", printer];
    args.extend_from_slice(options);
    args.push("-");
    Ok(parse_request(&cups("lp", &args, Some(data))?, printer))
}

/// The CUPS printers, then the label printers of `config.json`.
#[tauri::command]
pub async fn printer_list(app: AppHandle) -> Result<Vec<Printer>, String> {
    let labels = load_config(&app)?.label_printers;
    tauri::async_runtime::spawn_blocking(move || {
        let destinations = cups_printers()?;
        // `lpstat -d` can fail when no destination exists: that is no default, not an error.
        let default = cups("lpstat", &["-d"], None).ok().and_then(|text| parse_default(&text, &destinations));
        let mut printers: Vec<Printer> = destinations
            .into_iter()
            .map(|name| Printer { default: default.as_deref() == Some(name.as_str()), name, kind: Kind::Cups })
            .collect();
        printers.extend(labels.into_keys().map(|name| Printer { name, kind: Kind::Label, default: false }));
        Ok(printers)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Prints `pdf_base64` on the CUPS printer `printer`, `copies` copies (default 1), with no dialog. Returns the
/// CUPS request id.
#[tauri::command]
pub async fn print_pdf(printer: String, pdf_base64: String, copies: Option<u32>) -> Result<String, String> {
    let pdf = STANDARD.decode(pdf_base64.trim()).map_err(|e| format!("Il PDF non è base64: {e}"))?;
    if !pdf.starts_with(b"%PDF-") {
        return Err("Il documento non è un PDF.".into());
    }
    let copies = copies.unwrap_or(1);
    if copies == 0 {
        return Err("Il numero di copie deve essere almeno 1.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        if !cups_printers()?.contains(&printer) {
            return Err(format!("Stampante non trovata: {printer}."));
        }
        let copies = copies.to_string();
        send_lp(&printer, &["-n", &copies, "-o", "fit-to-page"], &pdf)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Sends `zpl` unchanged to the label printer `printer` of `config.json`.
#[tauri::command]
pub async fn print_zpl(app: AppHandle, printer: String, zpl: String) -> Result<(), String> {
    let address = load_config(&app)?
        .label_printers
        .remove(&printer)
        .ok_or_else(|| format!("Stampante di etichette non configurata: {printer}."))?;
    let target = label_target(&address)?;
    tauri::async_runtime::spawn_blocking(move || match target {
        LabelTarget::Tcp(host, port) => {
            let address = (host.as_str(), port)
                .to_socket_addrs()
                .map_err(|e| format!("{host}: {e}"))?
                .next()
                .ok_or_else(|| format!("{host}: indirizzo non trovato."))?;
            let mut stream =
                TcpStream::connect_timeout(&address, ZPL_TIMEOUT).map_err(|e| format!("{printer} ({address}): {e}"))?;
            stream.set_write_timeout(Some(ZPL_TIMEOUT)).map_err(|e| e.to_string())?;
            stream.write_all(zpl.as_bytes()).and_then(|_| stream.flush()).map_err(|e| format!("{printer}: {e}"))
        }
        LabelTarget::Queue(queue) => send_lp(&queue, &["-o", "raw"], zpl.as_bytes()).map(|_| ()),
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_and_default_come_from_lpstat() {
        assert_eq!(parse_destinations("HP_LaserJet\nZebra_ZD421\n\n"), vec!["HP_LaserJet", "Zebra_ZD421"]);
        let destinations = vec!["HP_LaserJet".to_string()];
        assert_eq!(parse_default("system default destination: HP_LaserJet\n", &destinations), Some("HP_LaserJet".into()));
        assert_eq!(parse_default("destinazione predefinita di sistema: HP_LaserJet\n", &destinations), Some("HP_LaserJet".into()));
        assert_eq!(parse_default("nessuna destinazione predefinita di sistema\n", &destinations), None);
        assert_eq!(parse_default("system default destination: Other\n", &destinations), None);
    }

    #[test]
    fn the_request_id_comes_from_lp() {
        assert_eq!(parse_request("request id is HP_LaserJet-42 (1 file(s))\n", "HP_LaserJet"), "HP_LaserJet-42");
        assert_eq!(parse_request("l'ID richiesta è HP_LaserJet-42 (1 file)\n", "HP_LaserJet"), "HP_LaserJet-42");
    }

    #[test]
    fn a_label_printer_is_a_tcp_address_or_a_queue() {
        assert_eq!(label_target("tcp://192.168.1.50").unwrap(), LabelTarget::Tcp("192.168.1.50".into(), 9100));
        assert_eq!(label_target("tcp://zebra.local:6101").unwrap(), LabelTarget::Tcp("zebra.local".into(), 6101));
        assert_eq!(label_target("tcp://[fe80::1]").unwrap(), LabelTarget::Tcp("fe80::1".into(), 9100));
        assert_eq!(label_target("tcp://[fe80::1]:9101").unwrap(), LabelTarget::Tcp("fe80::1".into(), 9101));
        assert_eq!(label_target("Zebra_ZD421").unwrap(), LabelTarget::Queue("Zebra_ZD421".into()));
        assert!(label_target("tcp://").is_err());
        assert!(label_target("tcp://zebra:abc").is_err());
    }
}
