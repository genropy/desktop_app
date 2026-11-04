fn main() {
    // Each app command gets an `allow-<command>` permission; windows get only the ones
    // granted in capabilities/ (local windows) or at runtime (the GenroPy origin).
    let manifest = tauri_build::AppManifest::new().commands(&[
        "get_url",
        "save_url",
        "pin_request",
        "pin_submit",
        "pin_cancel",
        "card_list",
        "card_sign",
        "card_close",
        "device_call",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest)).expect("failed to run tauri-build");
}
