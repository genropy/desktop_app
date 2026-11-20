fn main() {
    println!("cargo:rerun-if-env-changed=SOFTWELL_LINK_PUBLIC_KEY");
    // Each app command gets an `allow-<command>` permission; windows get only the ones
    // granted in capabilities/ (local windows) or at runtime (the GenroPy origin).
    let manifest = tauri_build::AppManifest::new().commands(&[
        "pending_connection",
        "inspect_connection",
        "activate_connection",
        "open_local",
        "add_site",
        "site_error",
        "saved_sites",
        "open_sites",
        "device_status",
        "device_register",
        "device_sign",
        "device_forget",
        "pin_request",
        "pin_submit",
        "pin_cancel",
        "card_list",
        "card_sign",
        "card_close",
        "card_test",
        "manage_features",
        "manage_sites",
        "manage_set_primary",
        "manage_forget",
        "manage_remove",
        "manage_test_key",
        "device_call",
        "fiscal_status",
        "fiscal_receipt",
        "fiscal_x_report",
        "fiscal_daily_closure",
        "manage_fiscal_get",
        "manage_fiscal_set",
        "manage_fiscal_test",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest)).expect("failed to run tauri-build");
}
