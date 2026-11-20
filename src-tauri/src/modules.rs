//! Python modules installed on this computer from the saved sites, loaded by the sidecar after its own.
//!
//! A saved site lists its modules at `/_tools/modules`: a JSON array of manifests, each in a `{payload, signature}`
//! envelope signed with the Softwell key of the RunDoc. The payload is
//!
//! ```json
//! {"format": "it.softwell.genroapp.module/v1", "name": "scale", "version": "1.2.0",
//!  "origins": ["https://app.acme.com"], "files": {"scale/__init__.py": "<sha256 hex>", ...}}
//! ```
//!
//! Each file is read at `/_tools/modules/<name>/<version>/<path>`. A module is installed only when the signature is
//! valid, the origin of the site is in `origins` and every file matches its hash. It is written to
//! `<app config dir>/modules/<name>/<version>/`, with the envelope as `manifest.json`. At every start of the sidecar
//! the signature and the hashes are checked again; a folder that fails is not loaded.
//!
//! `name` is the Python package of the module and the name it registers with `@module`. Only the origins of the
//! manifest may call it through `device_call`.
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, State, Url};

use crate::connector;
use crate::sidecar::Sidecar;
use crate::sites;

const FORMAT: &str = "it.softwell.genroapp.module/v1";
const INDEX_PATH: &str = "/_tools/modules";
const MANIFEST: &str = "manifest.json";
/// The modules of the sidecar itself: a downloaded module never takes their name.
const BUILTIN: [&str; 2] = ["system", "inbox"];
const MAX_INDEX: usize = 1024 * 1024;
const MAX_FILE: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    format: String,
    pub name: String,
    pub version: String,
    pub origins: Vec<String>,
    pub files: BTreeMap<String, String>,
}

/// A module whose folder passed the checks at the last start of the sidecar.
#[derive(Clone, Debug, Serialize)]
pub struct Installed {
    pub name: String,
    pub version: String,
    pub origins: Vec<String>,
    #[serde(skip)]
    pub path: PathBuf,
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !BUILTIN.contains(&name)
}

fn valid_version(version: &str) -> bool {
    let mut chars = version.chars();
    version.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

/// A relative path with `/` separators, no empty, `.` or `..` component and no character Windows refuses.
fn valid_path(path: &str) -> bool {
    path != MANIFEST
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.chars().any(|c| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        })
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Checks the signature of `envelope` with `key` and the content of its manifest.
pub fn verify_manifest(envelope: &str, key: &str) -> Result<Manifest, String> {
    let payload = connector::signed_payload(envelope, key)?;
    let manifest: Manifest = serde_json::from_slice(&payload).map_err(|_| "Manifest del modulo non valido.")?;
    if manifest.format != FORMAT {
        return Err(format!("Formato del modulo non supportato: {}.", manifest.format));
    }
    if !valid_name(&manifest.name) {
        return Err(format!("Nome del modulo non valido: {}.", manifest.name));
    }
    if !valid_version(&manifest.version) {
        return Err(format!("Versione del modulo {} non valida.", manifest.name));
    }
    if manifest.files.is_empty() {
        return Err(format!("Il modulo {} non ha file.", manifest.name));
    }
    for (path, hash) in &manifest.files {
        if !valid_path(path) || !valid_hash(hash) {
            return Err(format!("Il modulo {} contiene un file non valido: {path}.", manifest.name));
        }
    }
    for origin in &manifest.origins {
        let url = Url::parse(origin).map_err(|_| format!("Origine non valida nel modulo {}: {origin}.", manifest.name))?;
        if url.scheme() != "https" || url.origin().ascii_serialization() != *origin {
            return Err(format!("Origine non valida nel modulo {}: {origin}.", manifest.name));
        }
    }
    Ok(manifest)
}

/// The site `origin` may install and call the module.
pub fn check_origin(manifest: &Manifest, origin: &str) -> Result<(), String> {
    if manifest.origins.iter().any(|allowed| allowed == origin) {
        Ok(())
    } else {
        Err(format!("Il modulo {} non è autorizzato per {origin}.", manifest.name))
    }
}

/// Writes the module to `root/<name>/<version>/` after checking every file against the manifest.
/// The other versions of the same module are removed.
pub fn install(root: &Path, envelope: &str, manifest: &Manifest, files: &BTreeMap<String, Vec<u8>>) -> Result<PathBuf, String> {
    if files.len() != manifest.files.len() {
        return Err(format!("Il modulo {} non ha i file del manifest.", manifest.name));
    }
    for (path, hash) in &manifest.files {
        let data = files.get(path).ok_or_else(|| format!("Manca il file {path} del modulo {}.", manifest.name))?;
        if sha256_hex(data) != *hash {
            return Err(format!("Il file {path} del modulo {} non corrisponde al manifest.", manifest.name));
        }
    }
    let staging = root.join(format!(".staging-{}-{}", manifest.name, manifest.version));
    remove_dir(&staging)?;
    let write = || -> std::io::Result<()> {
        for (path, data) in files {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap())?;
            fs::write(target, data)?;
        }
        fs::write(staging.join(MANIFEST), envelope)
    };
    if let Err(e) = write() {
        let _ = remove_dir(&staging);
        return Err(format!("Impossibile scrivere il modulo {}: {e}", manifest.name));
    }
    let folder = root.join(&manifest.name);
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let target = folder.join(&manifest.version);
    remove_dir(&target)?;
    fs::rename(&staging, &target).map_err(|e| format!("Impossibile installare il modulo {}: {e}", manifest.name))?;
    for old in fs::read_dir(&folder).map_err(|e| e.to_string())?.flatten() {
        if old.file_name().to_str() != Some(manifest.version.as_str()) {
            remove_dir(&old.path())?;
        }
    }
    Ok(target)
}

fn remove_dir(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(format!("Impossibile cancellare {}: {e}", path.display())),
        _ => Ok(()),
    }
}

/// Every file under `dir`, as paths relative to it with `/` separators.
fn list_files(dir: &Path, prefix: &str, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        if entry.file_type()?.is_dir() {
            list_files(&entry.path(), &relative, out)?;
        } else {
            out.push(relative);
        }
    }
    Ok(())
}

/// Checks an installed folder `root/<name>/<version>/`: signature, name and version of the folder, the hash of every
/// file, and no file outside the manifest.
pub fn verify_installed(folder: &Path, key: &str) -> Result<Installed, String> {
    let envelope = fs::read_to_string(folder.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let manifest = verify_manifest(&envelope, key)?;
    let version = folder.file_name().and_then(|n| n.to_str());
    let name = folder.parent().and_then(Path::file_name).and_then(|n| n.to_str());
    if name != Some(manifest.name.as_str()) || version != Some(manifest.version.as_str()) {
        return Err("La cartella non corrisponde al manifest.".into());
    }
    let mut present = Vec::new();
    list_files(folder, "", &mut present).map_err(|e| e.to_string())?;
    present.retain(|path| path != MANIFEST);
    if present.len() != manifest.files.len() {
        return Err("La cartella contiene file che il manifest non elenca.".into());
    }
    for path in present {
        let hash = manifest.files.get(&path).ok_or_else(|| format!("File non elencato nel manifest: {path}."))?;
        let data = fs::read(folder.join(&path)).map_err(|e| format!("{path}: {e}"))?;
        if sha256_hex(&data) != *hash {
            return Err(format!("Il file {path} non corrisponde al manifest."));
        }
    }
    Ok(Installed { name: manifest.name, version: manifest.version, origins: manifest.origins, path: folder.to_path_buf() })
}

/// The modules under `root` that pass `verify_installed`. A folder that fails is reported on stderr and skipped.
pub fn scan(root: &Path, key: &str) -> Vec<Installed> {
    let mut found = Vec::new();
    let Ok(names) = fs::read_dir(root) else { return found };
    for name in names.flatten().filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.')) {
        let Ok(versions) = fs::read_dir(name.path()) else { continue };
        for version in versions.flatten().filter(|e| e.path().is_dir()) {
            match verify_installed(&version.path(), key) {
                Ok(installed) => found.push(installed),
                Err(e) => eprintln!("[runner] module {} not loaded: {e}", version.path().display()),
            }
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

pub fn root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|e| e.to_string())?.join("modules"))
}

/// The verified modules of this computer, or none in a build without the Softwell key.
pub fn load(app: &AppHandle) -> Vec<Installed> {
    match (root(app), connector::public_key()) {
        (Ok(root), Ok(key)) => scan(&root, key),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("[runner] no downloaded modules: {e}");
            Vec::new()
        }
    }
}

/// `device_call` from `origin` may reach `module`: a downloaded module only from the origins of its manifest,
/// a module of the sidecar itself from every origin that received `device_call`.
pub fn may_call(installed: &[Installed], module: &str, origin: &str) -> Result<(), String> {
    match installed.iter().find(|m| m.name == module) {
        Some(m) if !m.origins.iter().any(|allowed| allowed == origin) => {
            Err(format!("Il modulo {module} non è autorizzato per {origin}."))
        }
        _ => Ok(()),
    }
}

// ------------------------------------------------------------------ download

fn client() -> Result<reqwest::Client, String> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

/// GET `url`; `None` for 404, the body up to `limit` bytes otherwise.
async fn get(client: &reqwest::Client, url: Url, limit: usize) -> Result<Option<Vec<u8>>, String> {
    let response = client.get(url.clone()).send().await.map_err(|e| format!("{url}: {e}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("{url}: {}", response.status()));
    }
    let body = response.bytes().await.map_err(|e| format!("{url}: {e}"))?;
    if body.len() > limit {
        return Err(format!("{url}: troppo grande"));
    }
    Ok(Some(body.to_vec()))
}

/// Downloads and installs the modules `origin` offers that are not installed yet in the same version.
/// Returns the names of the modules installed now. A site without `/_tools/modules` offers none.
pub async fn refresh(root: &Path, key: &str, origin: &Url) -> Result<Vec<String>, String> {
    let client = client()?;
    let site = origin.origin().ascii_serialization();
    let Some(index) = get(&client, origin.join(INDEX_PATH).map_err(|e| e.to_string())?, MAX_INDEX).await? else {
        return Ok(Vec::new());
    };
    let envelopes: Vec<serde_json::Value> =
        serde_json::from_slice(&index).map_err(|_| format!("{site}{INDEX_PATH}: elenco dei moduli non valido."))?;
    let mut installed = Vec::new();
    for value in envelopes {
        let envelope = value.to_string();
        let manifest = verify_manifest(&envelope, key)?;
        check_origin(&manifest, &site)?;
        if root.join(&manifest.name).join(&manifest.version).join(MANIFEST).is_file() {
            continue;
        }
        let mut files = BTreeMap::new();
        for path in manifest.files.keys() {
            let mut url = origin.join(INDEX_PATH).map_err(|e| e.to_string())?;
            url.path_segments_mut()
                .map_err(|_| "indirizzo del sito non valido")?
                .extend([manifest.name.as_str(), manifest.version.as_str()])
                .extend(path.split('/'));
            let data = get(&client, url, MAX_FILE).await?.ok_or_else(|| format!("Manca il file {path} del modulo {}.", manifest.name))?;
            files.insert(path.clone(), data);
        }
        install(root, &envelope, &manifest, &files)?;
        installed.push(format!("{} {}", manifest.name, manifest.version));
    }
    Ok(installed)
}

/// Downloads the modules of the saved sites `origins` and restarts the sidecar when one was installed.
/// Returns the modules installed and the errors, one line each.
async fn update(app: &AppHandle, origins: Vec<Url>) -> Result<(Vec<String>, Vec<String>), String> {
    let sidecar = app.state::<Sidecar>();
    let _updating = sidecar.updating.lock().await;
    let root = root(app)?;
    let key = connector::public_key()?;
    let (mut installed, mut errors) = (Vec::new(), Vec::new());
    for origin in origins {
        match refresh(&root, key, &origin).await {
            Ok(names) => installed.extend(names),
            Err(e) => errors.push(format!("{}: {e}", origin.origin().ascii_serialization())),
        }
    }
    if !installed.is_empty() {
        sidecar.restart()?;
    }
    Ok((installed, errors))
}

/// Downloads in the background the modules the saved site `url` offers. Called when its window opens.
pub fn update_in_background(app: &AppHandle, url: &Url) {
    let (app, url) = (app.clone(), url.clone());
    tauri::async_runtime::spawn(async move {
        match update(&app, vec![url]).await {
            Ok((installed, errors)) => {
                for name in installed {
                    eprintln!("[runner] module installed: {name}");
                }
                for error in errors {
                    eprintln!("[runner] modules: {error}");
                }
            }
            Err(e) => eprintln!("[runner] modules: {e}"),
        }
    });
}

// ------------------------------------------------------------------ window Gestione

#[tauri::command]
pub fn manage_modules(sidecar: State<'_, Sidecar>) -> Vec<Installed> {
    sidecar.installed()
}

/// Downloads the modules of every saved site.
#[tauri::command]
pub async fn manage_update_modules(app: AppHandle) -> Result<String, String> {
    let origins = sites::load(&app)?.iter().filter_map(|site| Url::parse(&site.origin()).ok()).collect();
    let (installed, errors) = update(&app, origins).await?;
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(if installed.is_empty() {
        "Nessun modulo nuovo.".into()
    } else {
        format!("Installati: {}.", installed.join(", "))
    })
}

#[tauri::command]
pub async fn manage_remove_module(app: AppHandle, name: String) -> Result<(), String> {
    if !valid_name(&name) {
        return Err(format!("Nome del modulo non valido: {name}."));
    }
    let sidecar = app.state::<Sidecar>();
    let _updating = sidecar.updating.lock().await;
    remove_dir(&root(&app)?.join(&name))?;
    sidecar.restart()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde_json::json;

    struct Signer {
        keys: Ed25519KeyPair,
    }

    impl Signer {
        fn new() -> Self {
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
            Self { keys: Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap() }
        }

        fn key(&self) -> String {
            STANDARD.encode(self.keys.public_key())
        }

        fn sign(&self, payload: &serde_json::Value) -> String {
            let bytes = serde_json::to_vec(payload).unwrap();
            json!({"payload": STANDARD.encode(&bytes), "signature": STANDARD.encode(self.keys.sign(&bytes))}).to_string()
        }
    }

    fn package() -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([
            ("scale/__init__.py".to_string(), b"from scale.device import Scale\n".to_vec()),
            ("scale/device.py".to_string(), b"class Scale(object):\n    pass\n".to_vec()),
        ])
    }

    fn payload(files: &BTreeMap<String, Vec<u8>>) -> serde_json::Value {
        let hashes: BTreeMap<_, _> = files.iter().map(|(path, data)| (path.clone(), sha256_hex(data))).collect();
        json!({"format": FORMAT, "name": "scale", "version": "1.0.0",
               "origins": ["https://app.acme.com"], "files": hashes})
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("genroapp-modules-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn only_a_softwell_signature_is_accepted() {
        let softwell = Signer::new();
        let other = Signer::new();
        let envelope = softwell.sign(&payload(&package()));
        assert_eq!(verify_manifest(&envelope, &softwell.key()).unwrap().name, "scale");
        assert!(verify_manifest(&envelope, &other.key()).is_err());
        let mut forged: serde_json::Value = serde_json::from_str(&envelope).unwrap();
        let mut changed = payload(&package());
        changed["origins"] = json!(["https://evil.example"]);
        forged["payload"] = json!(STANDARD.encode(serde_json::to_vec(&changed).unwrap()));
        assert!(verify_manifest(&forged.to_string(), &softwell.key()).is_err());
    }

    #[test]
    fn bad_names_paths_and_origins_are_refused() {
        let softwell = Signer::new();
        let refused = |change: &dyn Fn(&mut serde_json::Value)| {
            let mut value = payload(&package());
            change(&mut value);
            verify_manifest(&softwell.sign(&value), &softwell.key()).is_err()
        };
        assert!(refused(&|v| v["name"] = json!("system")));
        assert!(refused(&|v| v["name"] = json!("../scale")));
        assert!(refused(&|v| v["name"] = json!("Scale")));
        assert!(refused(&|v| v["version"] = json!("1/2")));
        assert!(refused(&|v| v["files"]["../escape.py"] = json!(sha256_hex(b""))));
        assert!(refused(&|v| v["files"]["/abs.py"] = json!(sha256_hex(b""))));
        assert!(refused(&|v| v["files"]["manifest.json"] = json!(sha256_hex(b""))));
        assert!(refused(&|v| v["files"]["scale/x.py"] = json!("not-a-hash")));
        assert!(refused(&|v| v["origins"] = json!(["http://app.acme.com"])));
        assert!(refused(&|v| v["origins"] = json!(["https://app.acme.com/path"])));
    }

    #[test]
    fn a_site_outside_the_manifest_is_refused() {
        let softwell = Signer::new();
        let manifest = verify_manifest(&softwell.sign(&payload(&package())), &softwell.key()).unwrap();
        assert!(check_origin(&manifest, "https://app.acme.com").is_ok());
        assert!(check_origin(&manifest, "https://other.acme.com").is_err());
    }

    #[test]
    fn a_file_that_differs_from_its_hash_is_not_installed() {
        let softwell = Signer::new();
        let root = temp_root("hash");
        let envelope = softwell.sign(&payload(&package()));
        let manifest = verify_manifest(&envelope, &softwell.key()).unwrap();
        let mut files = package();
        files.insert("scale/device.py".into(), b"import os; os.system('x')\n".to_vec());
        assert!(install(&root, &envelope, &manifest, &files).is_err());
        assert!(!root.join("scale").exists());
        let mut missing = package();
        missing.remove("scale/device.py");
        assert!(install(&root, &envelope, &manifest, &missing).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_installed_module_is_checked_again_at_start() {
        let softwell = Signer::new();
        let root = temp_root("installed");
        let envelope = softwell.sign(&payload(&package()));
        let manifest = verify_manifest(&envelope, &softwell.key()).unwrap();
        let folder = install(&root, &envelope, &manifest, &package()).unwrap();
        assert_eq!(folder, root.join("scale").join("1.0.0"));
        let found = scan(&root, &softwell.key());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].origins, vec!["https://app.acme.com".to_string()]);

        fs::write(folder.join("scale").join("extra.py"), b"").unwrap();
        assert!(scan(&root, &softwell.key()).is_empty());
        fs::remove_file(folder.join("scale").join("extra.py")).unwrap();
        fs::write(folder.join("scale").join("device.py"), b"changed\n").unwrap();
        assert!(scan(&root, &softwell.key()).is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_new_version_replaces_the_old_one() {
        let softwell = Signer::new();
        let root = temp_root("version");
        let first = softwell.sign(&payload(&package()));
        install(&root, &first, &verify_manifest(&first, &softwell.key()).unwrap(), &package()).unwrap();
        let mut value = payload(&package());
        value["version"] = json!("1.1.0");
        let second = softwell.sign(&value);
        install(&root, &second, &verify_manifest(&second, &softwell.key()).unwrap(), &package()).unwrap();
        let found = scan(&root, &softwell.key());
        assert_eq!(found.iter().map(|m| m.version.as_str()).collect::<Vec<_>>(), ["1.1.0"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_downloaded_module_answers_only_its_origins() {
        let installed = vec![Installed {
            name: "scale".into(),
            version: "1.0.0".into(),
            origins: vec!["https://app.acme.com".into()],
            path: PathBuf::new(),
        }];
        assert!(may_call(&installed, "scale", "https://app.acme.com").is_ok());
        assert!(may_call(&installed, "scale", "https://other.acme.com").is_err());
        assert!(may_call(&installed, "system", "https://other.acme.com").is_ok());
    }
}
