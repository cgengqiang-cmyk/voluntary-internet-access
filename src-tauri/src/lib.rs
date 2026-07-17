mod commands;
pub mod config;
mod core;
mod credentials;
mod error;
mod fetch;
pub mod helper;
mod helper_install;
mod logging;
mod model;
pub mod network;
mod paths;
mod profile;
mod state;
mod tray;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .setup(|app| {
            let version = app.package_info().version.to_string();
            let state = state::AppState::initialize(app.handle(), &version)?;
            app.manage(state);
            tray::install(app)?;
            let started_by_autostart =
                std::env::args_os().any(|argument| argument == "--autostart");
            if started_by_autostart && let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                commands::auto_connect_after_startup(handle).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_snapshot,
            commands::set_connection,
            commands::set_transport_mode,
            commands::set_proxy_mode,
            commands::import_subscription,
            commands::import_profile_file,
            commands::refresh_profile,
            commands::select_proxy,
            commands::test_proxy_delay,
            commands::update_settings,
            commands::export_diagnostics,
            commands::repair_network,
            commands::uninstall_components,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
