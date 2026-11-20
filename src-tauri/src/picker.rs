//! Modeless macOS document chooser. Only accessed on the AppKit main thread.
use block2::RcBlock;
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{NSArray, NSString};
use std::cell::RefCell;
use std::path::Path;
use tauri::AppHandle;

thread_local! {
    static PANEL: RefCell<Option<Retained<NSOpenPanel>>> = const { RefCell::new(None) };
}

pub fn dismiss() {
    let panel = PANEL.with(|slot| slot.borrow_mut().take());
    if let Some(panel) = panel {
        // Standard AppKit cancel action without a sender.
        unsafe { panel.cancel(None) };
    }
}

pub fn show(app: &AppHandle) -> tauri::Result<()> {
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if let Some(panel) = PANEL.with(|slot| slot.borrow().clone()) {
            panel.makeKeyAndOrderFront(None);
            return;
        }
        let panel = NSOpenPanel::openPanel(MainThreadMarker::new().expect("AppKit main thread"));
        panel.setTitle(Some(&NSString::from_str("Apri RunDoc")));
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(true);
        #[allow(deprecated)]
        panel.setAllowedFileTypes(Some(&NSArray::from_slice(&[
            &*NSString::from_str("rundoc"),
            &*NSString::from_str("softwell"),
        ])));
        PANEL.with(|slot| slot.replace(Some(panel.clone())));
        let chosen = panel.clone();
        panel.beginWithCompletionHandler(&RcBlock::new(move |response| {
            PANEL.with(|slot| {
                if slot.borrow().as_ref() == Some(&chosen) {
                    slot.replace(None);
                }
            });
            if response == NSModalResponseOK {
                for url in chosen.URLs().iter() {
                    if let Some(path) = url.path() {
                        super::handle_document(&handle, Path::new(&path.to_string()));
                    }
                }
            }
        }));
    })
}
