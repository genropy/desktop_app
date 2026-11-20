//! The sites the user added by address, kept in `<app config dir>/sites.json`.
//!
//! A site is added only when it serves, at `/_tools/rundoc`, a RunDoc signed by Softwell whose URL has the origin
//! the user typed. The file belongs to the account of the operating system: another account starts with no sites.
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};

use crate::connector::{self, Connection};
use crate::documents::{self, Document};

pub const WINDOW: &str = "addsite";
/// The window that lists the saved sites at start, when there are several and none is primary.
pub const CHOOSER: &str = "sites";
const RUNDOC_PATH: &str = "/_tools/rundoc";
const NOT_ENABLED: &str = "Questo sito non è abilitato a GenroApp.";

/// One saved site. `user`, `key_id` and `verification` are set when this computer is registered on the site.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Site {
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
    /// The site GenroApp opens at start when several are saved. At most one site has it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub primary: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// What GenroApp shows at start, from the saved sites.
#[derive(Debug, PartialEq)]
pub enum Start {
    /// No site saved: the window "Aggiungi sito".
    AddSite,
    /// The only saved site, or the primary one: its origin.
    Open(String),
    /// Several sites and no primary: the list to choose from.
    Choose,
}

pub fn start(sites: &[Site]) -> Start {
    match sites {
        [] => Start::AddSite,
        [site] => Start::Open(site.origin()),
        _ => match sites.iter().find(|site| site.primary) {
            Some(site) => Start::Open(site.origin()),
            None => Start::Choose,
        },
    }
}

/// Makes the site of `origin` the primary one, or no site primary when it already is.
fn toggle_primary(sites: &mut [Site], origin: &str) {
    let was_primary = sites.iter().any(|site| site.primary && site.origin() == origin);
    for site in sites.iter_mut() {
        site.primary = !was_primary && site.origin() == origin;
    }
}

impl Site {
    /// The origin of `url`, as `https://host[:port]`.
    pub fn origin(&self) -> String {
        Url::parse(&self.url).map(|url| url.origin().ascii_serialization()).unwrap_or_default()
    }

    pub fn document(&self) -> Document {
        Document { name: self.name.clone(), url: self.url.clone(), id: self.id.clone(), icon: self.icon.clone() }
    }

    /// Name, URL, id and icon from `link`; the registration of the computer stays.
    fn update_from(&mut self, link: &Connection) {
        self.name = link.name.clone();
        self.url = link.url.clone();
        self.id = link.id.clone();
        self.icon = link.icon.clone();
    }
}

/// The message of a failed opening of a saved site, shown by the window "Aggiungi sito".
#[derive(Default)]
pub struct SiteError(Mutex<Option<String>>);

pub fn path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|e| e.to_string())?.join("sites.json"))
}

/// The sites of `path`; none when the file does not exist.
pub fn read(path: &Path) -> Result<Vec<Site>, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

pub fn write(path: &Path, sites: &[Site]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(sites).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load(app: &AppHandle) -> Result<Vec<Site>, String> {
    read(&path(app)?)
}

/// The saved site of `origin`.
pub fn find(app: &AppHandle, origin: &str) -> Result<Option<Site>, String> {
    Ok(load(app)?.into_iter().find(|site| site.origin() == origin))
}

/// Changes the saved site of `origin` with `change` and writes the file.
pub fn update(app: &AppHandle, origin: &str, change: impl FnOnce(&mut Site)) -> Result<Site, String> {
    let path = path(app)?;
    let mut sites = read(&path)?;
    let site = sites
        .iter_mut()
        .find(|site| site.origin() == origin)
        .ok_or_else(|| format!("Il sito {origin} non è fra quelli aggiunti a GenroApp."))?;
    change(site);
    let changed = site.clone();
    write(&path, &sites)?;
    Ok(changed)
}

/// Makes the site of `origin` the primary one, or none when it already is.
pub fn set_primary(app: &AppHandle, origin: &str) -> Result<(), String> {
    let path = path(app)?;
    let mut sites = read(&path)?;
    toggle_primary(&mut sites, origin);
    write(&path, &sites)
}

/// Adds the site of `link` to `sites`, or updates the one with the same origin.
fn upsert(sites: &mut Vec<Site>, link: &Connection) -> Site {
    let origin = Url::parse(&link.url).map(|url| url.origin().ascii_serialization()).unwrap_or_default();
    match sites.iter_mut().find(|site| site.origin() == origin) {
        Some(site) => {
            site.update_from(link);
            site.clone()
        }
        None => {
            let mut site = Site::default();
            site.update_from(link);
            sites.push(site.clone());
            site
        }
    }
}

/// The origin of the address the user typed: `app.acme.com`, `app.acme.com:8443` or `https://app.acme.com/`.
pub fn typed_origin(text: &str) -> Result<Url, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Scrivi l'indirizzo del sito.".into());
    }
    let with_scheme = if text.contains("://") { text.to_string() } else { format!("https://{text}") };
    let url = Url::parse(&with_scheme).map_err(|_| "Indirizzo non valido.")?;
    if url.scheme() != "https" || !url.has_host() || !url.username().is_empty() || url.password().is_some() {
        return Err("L'indirizzo deve essere un sito HTTPS.".into());
    }
    Url::parse(&url.origin().ascii_serialization()).map_err(|_| "Indirizzo non valido.".into())
}

/// The RunDoc must name a URL with the same origin as the address typed: a RunDoc copied onto another site is refused.
pub fn check_origin(typed: &Url, link: &Connection) -> Result<(), String> {
    let url = Url::parse(&link.url).map_err(|_| NOT_ENABLED)?;
    if url.origin() != typed.origin() {
        return Err(NOT_ENABLED.into());
    }
    Ok(())
}

/// Reads `/_tools/rundoc` of `origin`, verifies the Softwell signature and the origin.
pub async fn fetch(origin: &Url) -> Result<Connection, String> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let address = origin.join(RUNDOC_PATH).map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(address)
        .send()
        .await
        .map_err(|e| format!("Impossibile raggiungere {}: {e}", origin.origin().ascii_serialization()))?;
    if !response.status().is_success() {
        return Err(NOT_ENABLED.into());
    }
    let body = response.bytes().await.map_err(|e| e.to_string())?;
    if body.len() > connector::MAX_SIZE {
        return Err(NOT_ENABLED.into());
    }
    let text = std::str::from_utf8(&body).map_err(|_| NOT_ENABLED)?;
    let link = connector::verify(text).map_err(|_| NOT_ENABLED.to_string())?;
    check_origin(origin, &link)?;
    Ok(link)
}

/// Reads and checks the RunDoc of `origin`, saves the site and returns it.
async fn fetch_and_save(app: &AppHandle, origin: &Url) -> Result<Site, String> {
    let link = fetch(origin).await?;
    let path = path(app)?;
    let mut sites = read(&path)?;
    let site = upsert(&mut sites, &link);
    write(&path, &sites)?;
    Ok(site)
}

pub fn open_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(WINDOW) {
        window.unminimize()?;
        return window.set_focus();
    }
    WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("addsite.html".into()))
        .title("Aggiungi sito")
        .inner_size(420.0, 200.0)
        .resizable(false)
        .build()?;
    Ok(())
}

fn close_window(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(WINDOW) {
        window.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Opens the saved site of `origin` from the menu: its RunDoc is read and checked again first.
/// On failure the window "Aggiungi sito" shows the message.
pub fn open_saved(app: &AppHandle, origin: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = match Url::parse(&origin) {
            Ok(url) => fetch_and_save(&app, &url).await,
            Err(e) => Err(e.to_string()),
        };
        let result = result.and_then(|site| documents::open(&app, site.document()).map_err(|e| e.to_string()));
        if let Err(error) = result {
            *app.state::<SiteError>().0.lock().unwrap() = Some(format!("{origin}: {error}"));
            let _ = close_window(&app);
            let _ = open_window(&app);
        }
    });
}

pub fn open_chooser(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(CHOOSER) {
        window.unminimize()?;
        return window.set_focus();
    }
    WebviewWindowBuilder::new(app, CHOOSER, WebviewUrl::App("sites.html".into()))
        .title("Apri siti")
        .inner_size(420.0, 320.0)
        .build()?;
    Ok(())
}

/// The window GenroApp shows when none is open: the saved site to open, the list, or "Aggiungi sito".
pub fn open_start(app: &AppHandle) -> tauri::Result<()> {
    match start(&load(app).unwrap_or_default()) {
        Start::AddSite => open_window(app),
        Start::Open(origin) => {
            open_saved(app, origin);
            Ok(())
        }
        Start::Choose => open_chooser(app),
    }
}

#[derive(Serialize)]
pub struct Entry {
    name: String,
    origin: String,
}

/// The saved sites, for the list of the window "Apri siti".
#[tauri::command]
pub fn saved_sites(app: AppHandle) -> Result<Vec<Entry>, String> {
    Ok(load(&app)?.into_iter().map(|site| Entry { origin: site.origin(), name: site.name }).collect())
}

/// Opens the saved sites of `origins` and closes the window "Apri siti".
#[tauri::command]
pub fn open_sites(app: AppHandle, origins: Vec<String>) -> Result<(), String> {
    if origins.is_empty() {
        return Err("Scegli almeno un sito.".into());
    }
    for origin in origins {
        open_saved(&app, origin);
    }
    if let Some(window) = app.get_webview_window(CHOOSER) {
        window.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn add_site(app: AppHandle, address: String) -> Result<(), String> {
    let origin = typed_origin(&address)?;
    let site = fetch_and_save(&app, &origin).await?;
    documents::open(&app, site.document()).map_err(|e| e.to_string())?;
    close_window(&app)
}

/// The message of the last failed opening of a saved site, once.
#[tauri::command]
pub fn site_error(state: State<'_, SiteError>) -> Option<String> {
    state.0.lock().unwrap().take()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(url: &str) -> Connection {
        serde_json::from_value(serde_json::json!({
            "format": "it.softwell.runner.link/v1", "name": "Studio", "url": url
        }))
        .unwrap()
    }

    #[test]
    fn the_typed_address_becomes_an_https_origin() {
        assert_eq!(typed_origin("app.acme.com").unwrap().as_str(), "https://app.acme.com/");
        assert_eq!(typed_origin(" localhost:8080 ").unwrap().as_str(), "https://localhost:8080/");
        assert_eq!(typed_origin("https://app.acme.com/x/y?z").unwrap().as_str(), "https://app.acme.com/");
        assert!(typed_origin("http://app.acme.com").is_err());
        assert!(typed_origin("https://u:p@app.acme.com").is_err());
        assert!(typed_origin("").is_err());
    }

    #[test]
    fn the_rundoc_must_name_the_typed_origin() {
        let typed = typed_origin("app.acme.com").unwrap();
        assert!(check_origin(&typed, &link("https://app.acme.com/")).is_ok());
        assert!(check_origin(&typed, &link("https://app.acme.com/studio/")).is_ok());
        assert!(check_origin(&typed, &link("https://other.acme.com/")).is_err());
        assert!(check_origin(&typed, &link("https://app.acme.com:8443/")).is_err());
        let local = typed_origin("localhost:8080").unwrap();
        assert!(check_origin(&local, &link("https://localhost:8080/")).is_ok());
        assert!(check_origin(&local, &link("https://localhost/")).is_err());
    }

    #[test]
    fn sites_are_stored_by_origin_and_keep_the_registration() {
        let dir = std::env::temp_dir().join(format!("genroapp-sites-{}", std::process::id()));
        let path = dir.join("sites.json");
        assert!(read(&path).unwrap().is_empty());

        let mut sites = Vec::new();
        upsert(&mut sites, &link("https://app.acme.com/"));
        upsert(&mut sites, &link("https://localhost:8080/"));
        sites[0].user = Some("mario".into());
        sites[0].key_id = Some("abc".into());
        sites[0].verification = Some("biometry".into());
        let mut renamed = link("https://app.acme.com/studio/");
        renamed.name = "Acme".into();
        upsert(&mut sites, &renamed);
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].name, "Acme");
        assert_eq!(sites[0].user.as_deref(), Some("mario"));
        assert_eq!(sites[0].origin(), "https://app.acme.com");

        write(&path, &sites).unwrap();
        assert_eq!(read(&path).unwrap(), sites);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_start_opens_the_only_or_the_primary_site() {
        let mut sites = Vec::new();
        assert_eq!(start(&sites), Start::AddSite);
        upsert(&mut sites, &link("https://app.acme.com/"));
        assert_eq!(start(&sites), Start::Open("https://app.acme.com".into()));
        upsert(&mut sites, &link("https://localhost:8080/"));
        assert_eq!(start(&sites), Start::Choose);

        toggle_primary(&mut sites, "https://localhost:8080");
        assert_eq!(start(&sites), Start::Open("https://localhost:8080".into()));
        toggle_primary(&mut sites, "https://app.acme.com");
        assert_eq!(sites.iter().filter(|site| site.primary).count(), 1);
        assert_eq!(start(&sites), Start::Open("https://app.acme.com".into()));
        toggle_primary(&mut sites, "https://app.acme.com");
        assert_eq!(start(&sites), Start::Choose);
    }
}
