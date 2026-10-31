use tauri::{Emitter, Manager};

pub fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(windows)]
        crate::double_alt::WINDOW_VISIBLE.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = window.show();
        let _ = window.set_focus();
        #[cfg(windows)]
        if let Ok(hwnd) = window.hwnd() {
            crate::force_foreground(hwnd.0 as isize);
        }
    }
}

#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn restart_app(app: tauri::AppHandle) {
    app.request_restart();
}

pub fn setup(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::{
        menu::{Menu, MenuItem, PredefinedMenuItem},
        tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    };
    let version = MenuItem::with_id(
        app,
        "version",
        format!("Luma {}", env!("CARGO_PKG_VERSION")),
        false,
        None::<&str>,
    )?;
    let show = MenuItem::with_id(app, "show", "显示 Luma", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "刷新应用", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let diagnostics = MenuItem::with_id(app, "diagnostics", "应用诊断", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, "restart", "重新启动", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Luma", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &version,
            &show,
            &refresh,
            &settings,
            &diagnostics,
            &separator,
            &restart,
            &quit,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id("luma-tray")
        .menu(&menu)
        .tooltip(format!("Luma {}", env!("CARGO_PKG_VERSION")))
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => self::show(app),
            "refresh" => {
                self::show(app);
                let _ = app.emit("refresh-apps", ());
            }
            "settings" => {
                self::show(app);
                let _ = app.emit("open-settings", ());
            }
            "diagnostics" => {
                self::show(app);
                let _ = app.emit("open-diagnostics", ());
            }
            "restart" => app.request_restart(),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } = event
            {
                self::show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    if let Err(error) = crate::watcher::start(app.handle().clone()) {
        crate::diagnostics::record("监听", "系统", None, error.to_string());
    }
    Ok(())
}
