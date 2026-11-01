//! 原生桌面模糊和窗口裁切：CSS backdrop-filter 本身无法采样 WebView 外的桌面。
use tauri::Manager;

#[cfg(windows)]
mod frame {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                GWL_STYLE, STYLESTRUCT, WM_NCACTIVATE, WM_NCDESTROY, WM_NCPAINT, WM_STYLECHANGING,
                WS_CAPTION,
            },
        },
    };
    const ID: usize = 0x4c554d47;
    unsafe extern "system" fn procedure(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        match message {
            WM_STYLECHANGING if wparam as isize == GWL_STYLE as isize && lparam != 0 => unsafe {
                (*(lparam as *mut STYLESTRUCT)).styleNew &= !WS_CAPTION;
            },
            WM_NCPAINT => return 0,
            // 保留 Tao 的活动/焦点同步；-1 只禁止默认非客户区重绘。
            WM_NCACTIVATE => return unsafe { DefSubclassProc(hwnd, message, wparam, -1) },
            WM_NCDESTROY => unsafe {
                RemoveWindowSubclass(hwnd, Some(procedure), ID);
            },
            _ => {}
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }
    pub fn attach(window: &tauri::WebviewWindow) -> Result<(), String> {
        let hwnd = window.hwnd().map_err(|e| e.to_string())?;
        if unsafe { SetWindowSubclass(hwnd.0 as _, Some(procedure), ID, 0) } == 0 {
            return Err("无法安装无边框玻璃监听".into());
        }
        Ok(())
    }
}

#[cfg(windows)]
fn remove_native_frame(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WS_CAPTION,
    };
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    unsafe {
        let style = GetWindowLongPtrW(hwnd.0 as _, GWL_STYLE);
        if style & WS_CAPTION as isize != 0 {
            // Tao 使用 NCCALCSIZE 模拟无边框，但仍保留 WS_CAPTION；DWM 玻璃会把它显出来。
            SetWindowLongPtrW(hwnd.0 as _, GWL_STYLE, style & !(WS_CAPTION as isize));
            if SetWindowPos(
                hwnd.0 as _,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            ) == 0
            {
                return Err("无法清除玻璃层下的系统标题栏".into());
            }
        }
    }
    Ok(())
}

pub fn after_show(window: &tauri::WebviewWindow) {
    #[cfg(windows)]
    if let Err(error) = remove_native_frame(window) {
        crate::diagnostics::record("窗口", "玻璃边框", None, error);
    }
}

#[cfg(windows)]
fn clip(window: &tauri::WebviewWindow) -> Result<(), String> {
    use std::sync::Mutex;
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn};
    static LAST: Mutex<Option<(i32, i32, i32)>> = Mutex::new(None);
    let size = window.outer_size().map_err(|e| e.to_string())?;
    if size.width == 0 || size.height == 0 {
        return Ok(());
    }
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let radius = (crate::config::get()
        .appearance
        .corner_radius
        .clamp(0.0, 48.0)
        * scale)
        .round() as i32;
    let shape = (
        size.width.min(i32::MAX as u32 - 1) as i32,
        size.height.min(i32::MAX as u32 - 1) as i32,
        radius,
    );
    {
        let mut last = LAST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *last == Some(shape) {
            return Ok(());
        }
        *last = Some(shape);
    }
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    unsafe {
        let region = CreateRoundRectRgn(0, 0, shape.0 + 1, shape.1 + 1, radius * 2, radius * 2);
        if region.is_null() || SetWindowRgn(hwnd.0 as _, region, 1) == 0 {
            if !region.is_null() {
                DeleteObject(region);
            }
            *LAST
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            return Err("无法更新玻璃窗口的圆角裁切".into());
        }
        // SetWindowRgn 成功后由系统接管 region，不能再次 DeleteObject。
    }
    Ok(())
}

pub fn setup(window: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        if let Err(error) = frame::attach(window) {
            crate::diagnostics::record("窗口", "玻璃边框", None, error);
        }
        let _ = window.set_theme(Some(tauri::Theme::Dark));
        after_show(window);
        match window_vibrancy::apply_acrylic(window, Some((18, 22, 38, 135))) {
            Ok(()) => crate::diagnostics::record(
                "窗口",
                "玻璃材质",
                None,
                "已启用 Windows 原生 Acrylic 桌面模糊",
            ),
            Err(error) => crate::diagnostics::record(
                "窗口",
                "玻璃材质",
                None,
                format!("使用深色玻璃回退：{error}"),
            ),
        }
        if let Err(error) = clip(window) {
            crate::diagnostics::record("窗口", "圆角裁切", None, error);
        }
        let target = window.clone();
        window.on_window_event(move |event| {
            if matches!(
                event,
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }
            ) {
                if let Err(error) = clip(&target) {
                    crate::diagnostics::record("窗口", "圆角裁切", None, error);
                }
            }
        });
    }
}

pub fn refresh(app: &tauri::AppHandle) {
    #[cfg(windows)]
    if let Some(window) = app.get_webview_window("main") {
        let target = window.clone();
        let _ = window.run_on_main_thread(move || {
            if let Err(error) = clip(&target) {
                crate::diagnostics::record("窗口", "圆角裁切", None, error);
            }
        });
    }
}
