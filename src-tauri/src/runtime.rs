use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use tauri::{Emitter, Manager};
static NATIVE_DIALOGS: AtomicUsize = AtomicUsize::new(0);
static OVERLAY: AtomicBool = AtomicBool::new(false);
static ESC_QUIET_UNTIL: AtomicU64 = AtomicU64::new(0);
fn elapsed_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}
fn quiet_esc() {
    ESC_QUIET_UNTIL.store(elapsed_ms().saturating_add(300), Ordering::Release);
}
pub fn esc_should_hide() -> bool {
    !overlay_open()
        && !native_dialog_open()
        && elapsed_ms() >= ESC_QUIET_UNTIL.load(Ordering::Acquire)
}
pub struct VisibilityHold;
pub fn hold_visible() -> VisibilityHold {
    NATIVE_DIALOGS.fetch_add(1, Ordering::AcqRel);
    VisibilityHold
}
impl Drop for VisibilityHold {
    fn drop(&mut self) {
        quiet_esc();
        NATIVE_DIALOGS.fetch_sub(1, Ordering::AcqRel);
    }
}
pub fn native_dialog_open() -> bool {
    NATIVE_DIALOGS.load(Ordering::Acquire) > 0
}
pub fn overlay_open() -> bool {
    OVERLAY.load(Ordering::Acquire)
}
#[tauri::command]
pub fn set_overlay_open(open: bool) {
    if OVERLAY.swap(open, Ordering::AcqRel) && !open {
        quiet_esc();
    }
}

pub fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        crate::position::before_show(&window);
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
    #[cfg(windows)]
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.with_webview(|webview| {
            use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings9;
            use windows::core::Interface;
            let result = (|| -> windows::core::Result<()> {
                unsafe {
                    let core = webview.controller().CoreWebView2()?;
                    let settings: ICoreWebView2Settings9 = core.Settings()?.cast()?;
                    settings.SetIsNonClientRegionSupportEnabled(true)?;
                    // setup 时页面仍为 about:blank，首次应用导航将使用此设置。
                }
                Ok(())
            })();
            match result {
                Ok(()) => {
                    crate::diagnostics::record("窗口", "拖动", None, "已启用原生标题栏拖动区域")
                }
                Err(error) => crate::diagnostics::record(
                    "窗口",
                    "拖动",
                    None,
                    format!("使用兼容拖动：{error}"),
                ),
            }
        }) {
            crate::diagnostics::record("窗口", "拖动", None, error.to_string());
        }
    }
    if let Err(error) = crate::position::setup(app) {
        crate::diagnostics::record("窗口", "位置监听", None, error);
    }
    if let Err(error) = crate::watcher::start(app.handle().clone()) {
        crate::diagnostics::record("监听", "系统", None, error.to_string());
    }
    Ok(())
}
