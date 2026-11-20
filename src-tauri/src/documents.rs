use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Mutex;
use tauri::{image::Image, AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};

#[derive(Clone, Deserialize, Serialize)]
pub struct Document {
    pub name: String,
    pub url: String,
    pub id: Option<String>,
    pub icon: Option<String>,
}

impl Document {
    pub fn from_url(url: &Url) -> Self {
        Self {
            name: url.host_str().unwrap_or("Gestionale").into(),
            url: url.to_string(),
            id: None,
            icon: None,
        }
    }

    pub fn label(&self, url: &Url) -> String {
        let mut digest = Sha256::new();
        digest.update(url.origin().ascii_serialization());
        digest.update([0]);
        digest.update(self.id.as_deref().unwrap_or(url.as_str()));
        format!("doc-{}", digest.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>())
    }

    pub fn image(&self) -> Option<Image<'static>> {
        let bytes = self.icon.as_ref().and_then(|s| STANDARD.decode(s).ok());
        Image::from_bytes(
            bytes
                .as_deref()
                .unwrap_or(include_bytes!("../icons/32x32.png")),
        )
        .ok()
    }
}

#[derive(Default)]
pub struct Windows {
    pub open: BTreeMap<String, Document>,
    pub active: Option<String>,
}
pub type Registry = Mutex<Windows>;

pub fn focus(app: &AppHandle, label: &str) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(label) {
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
    }
    Ok(())
}

pub fn open(app: &AppHandle, doc: Document) -> tauri::Result<()> {
    let url = Url::parse(&doc.url).map_err(std::io::Error::other)?;
    let label = doc.label(&url);
    if app.get_webview_window(&label).is_some() {
        return focus(app, &label);
    }
    #[cfg(feature = "devices")]
    if !super::local::is_local(&url) {
        super::devices::grant_card_commands(app, &url, &label)?;
    }
    if matches!(super::sites::find(app, &url.origin().ascii_serialization()), Ok(Some(_))) {
        super::device_login::grant(app, &url, &label)?;
    }
    WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title(&doc.name)
        .inner_size(1280.0, 800.0)
        .build()?;
    {
        let registry = app.state::<Registry>();
        let mut windows = registry.lock().unwrap();
        windows.open.insert(label.clone(), doc);
        windows.active = Some(label);
    }
    super::refresh_menu(app)
}

pub fn active(app: &AppHandle) -> Option<(String, Document)> {
    let registry = app.state::<Registry>();
    let windows = registry.lock().unwrap();
    let label = windows.active.as_ref()?;
    Some((label.clone(), windows.open.get(label)?.clone()))
}
