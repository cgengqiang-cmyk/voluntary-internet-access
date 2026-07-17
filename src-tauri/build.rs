#[cfg(target_os = "macos")]
#[path = "src/core/macho_integrity.rs"]
mod macho_integrity;

fn main() {
    #[cfg(target_os = "macos")]
    emit_macos_mihomo_content_hash();

    const COMMANDS: &[&str] = &[
        "get_app_snapshot",
        "set_connection",
        "set_transport_mode",
        "set_proxy_mode",
        "import_subscription",
        "import_profile_file",
        "refresh_profile",
        "select_proxy",
        "test_proxy_delay",
        "update_settings",
        "export_diagnostics",
        "repair_network",
        "uninstall_components",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to build VIA Tauri manifest");
}

#[cfg(target_os = "macos")]
fn emit_macos_mihomo_content_hash() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let core = manifest
        .join("binaries")
        .join("mihomo-aarch64-apple-darwin");
    println!("cargo:rerun-if-changed={}", core.display());
    let bytes = std::fs::read(&core).expect("read pinned macOS Mihomo for content hashing");
    let digest = macho_integrity::normalized_macho_sha256(&bytes)
        .expect("normalize pinned macOS Mihomo Mach-O content");
    println!("cargo:rustc-env=VIA_MACOS_MIHOMO_CONTENT_SHA256={digest}");
}
