fn main() {
    println!("cargo:rerun-if-env-changed=SOFTWELL_LINK_PUBLIC_KEY");
    // Each app command gets an `allow-<command>` permission; windows get only the ones
    // granted in capabilities/ (local windows) or at runtime (the GenroPy origin).
    let manifest = tauri_build::AppManifest::new().commands(&[
        "pending_connection",
        "inspect_connection",
        "activate_connection",
        "open_local",
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
