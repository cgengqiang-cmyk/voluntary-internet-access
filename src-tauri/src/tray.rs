use tauri::{
    App, AppHandle, Manager,
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::commands;

const MENU_OPEN: &str = "open";
const MENU_CONNECT: &str = "connect";
const MENU_DISCONNECT: &str = "disconnect";
const MENU_QUIT: &str = "quit";

pub fn install(app: &App) -> tauri::Result<()> {
    let menu = MenuBuilder::new(app)
        .text(MENU_OPEN, "打开 VIA")
        .separator()
        .text(MENU_CONNECT, "连接")
        .text(MENU_DISCONNECT, "断开并恢复网络")
        .separator()
        .text(MENU_QUIT, "安全退出")
        .build()?;

    let mut builder = TrayIconBuilder::with_id("via-tray")
        .menu(&menu)
        .tooltip("我自愿开启国际互联网访问")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN => show_main_window(app),
            MENU_CONNECT => run_connection_action(app.clone(), true, false),
            MENU_DISCONNECT => run_connection_action(app.clone(), false, false),
            MENU_QUIT => run_connection_action(app.clone(), false, true),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    let _tray = builder.build(app)?;
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn run_connection_action(app: AppHandle, enabled: bool, quit_after: bool) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<crate::state::AppState>();
        match commands::set_connection_inner(enabled, app.clone(), &state).await {
            Ok(_) if quit_after => app.exit(0),
            Ok(_) => {}
            Err(_) => show_main_window(&app),
        }
    });
}
