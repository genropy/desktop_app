//! A GenroPy instance on this machine, opened by port without a signed RunDoc.
//!
//! The host is fixed to `localhost` and the path to `/`: the port is the only input.
use tauri::Url;

use crate::documents::Document;

pub const WINDOW: &str = "local";
const HOST: &str = "localhost";

pub fn document(port: u16) -> Result<Document, String> {
    if port == 0 {
        return Err("Porta non valida.".into());
    }
    let url = Url::parse(&format!("http://{HOST}:{port}/")).map_err(|e| e.to_string())?;
    Ok(Document { name: format!("{HOST}:{port}"), url: url.to_string(), id: None, icon: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_port_comes_from_input() {
        let doc = document(8080).unwrap();
        assert_eq!(doc.url, "http://localhost:8080/");
        assert_eq!(doc.name, "localhost:8080");
        assert!(document(0).is_err());
    }
}
