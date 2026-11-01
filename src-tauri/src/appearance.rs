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
    #[cfg(windows)]
    if let Err(error) = clip(window).and_then(|()| verify_clip(window)) {
        crate::diagnostics::record("窗口", "圆角核验", None, error);
    }
}

#[cfg(windows)]
fn verify_clip(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows_sys::Win32::Graphics::Gdi::{
        CreateRectRgn, DeleteObject, GetRgnBox, GetWindowRgn, PtInRegion,
    };
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    unsafe {
        // Read a copy of the region actually held by Windows, not the region we requested.
        let copy = CreateRectRgn(0, 0, 0, 0);
        if copy.is_null() {
            return Err("无法创建圆角核验区域".into());
        }
        let mut bounds = std::mem::zeroed();
        let present = GetWindowRgn(hwnd.0 as _, copy) > 0;
        let valid_bounds = GetRgnBox(copy, &mut bounds) > 0
            && bounds.left == 0
            && bounds.top == 0
            && bounds.right == size.width as i32
            && bounds.bottom == size.height as i32;
        let w = size.width as i32;
        let h = size.height as i32;
        let corners = [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)]
            .map(|(x, y)| PtInRegion(copy, x, y) == 0);
        let center = PtInRegion(copy, w / 2, h / 2) != 0;
        DeleteObject(copy);
        let rounded = crate::config::get().appearance.corner_radius
            * window.scale_factor().unwrap_or(1.0)
            >= 8.0;
        if !present || !valid_bounds || !center || (rounded && !corners.iter().all(|value| *value))
        {
            return Err(format!(
                "原生区域核验失败：区域={present} 边界={valid_bounds} 中心={center} 四角={corners:?}"
            ));
        }
        crate::diagnostics::record(
            "窗口",
            "圆角核验",
            None,
            format!(
                "原生窗口区域 {}×{}，边界一致，中心可见，四角排除={corners:?}",
                w, h
            ),
        );
    }
    Ok(())
}

#[cfg(windows)]
fn rounded_region(width: i32, height: i32, radius: i32) -> windows_sys::Win32::Graphics::Gdi::HRGN {
    use windows_sys::Win32::Graphics::Gdi::{
        CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, RGN_AND,
    };
    // Expand the hard native curve by two physical pixels, then intersect it with
    // the exact window bounds. CSS owns the visible, antialiased edge.
    unsafe {
        let curve = CreateRoundRectRgn(-2, -2, width + 3, height + 3, radius * 2, radius * 2);
        let bounds = CreateRectRgn(0, 0, width, height);
        if curve.is_null() || bounds.is_null() {
            if !curve.is_null() {
                DeleteObject(curve);
            }
            if !bounds.is_null() {
                DeleteObject(bounds);
            }
            return std::ptr::null_mut();
        }
        let result = CombineRgn(curve, curve, bounds, RGN_AND);
        DeleteObject(bounds);
        if result == 0 {
            DeleteObject(curve);
            std::ptr::null_mut()
        } else {
            curve
        }
    }
}

#[cfg(windows)]
fn clip(window: &tauri::WebviewWindow) -> Result<(), String> {
    use std::sync::Mutex;
    use windows_sys::Win32::Graphics::Gdi::{DeleteObject, SetWindowRgn};
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
        let region = rounded_region(shape.0, shape.1, radius);
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

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn native_guard_preserves_css_antialiasing_coverage_in_all_corners() {
        use windows_sys::Win32::Graphics::Gdi::{DeleteObject, PtInRegion};
        for radius in [2, 6, 12, 24, 36, 48] {
            let (width, height) = (900, 640);
            unsafe {
                let region = super::rounded_region(width, height, radius);
                let mut clipped = Vec::new();
                for x in 0..radius {
                    for y in 0..radius {
                        let dx = x as f64 + 0.5 - radius as f64;
                        let dy = y as f64 + 0.5 - radius as f64;
                        // Include the partially covered CSS edge, not just fully opaque pixels.
                        if dx.hypot(dy) <= radius as f64 + 0.75 {
                            for (cx, cy) in [
                                (x, y),
                                (width - 1 - x, y),
                                (x, height - 1 - y),
                                (width - 1 - x, height - 1 - y),
                            ] {
                                if PtInRegion(region, cx, cy) == 0 {
                                    clipped.push((cx, cy));
                                }
                            }
                        }
                    }
                }
                DeleteObject(region);
                assert!(
                    clipped.is_empty(),
                    "radius={radius}, clipped CSS edge pixels: {clipped:?}"
                );
            }
        }
    }

    #[test]
    fn native_region_matches_window_bounds_and_excludes_only_rounded_corners() {
        use windows_sys::Win32::Graphics::Gdi::{DeleteObject, GetRgnBox, PtInRegion};
        for (width, height, radius) in [
            (900, 640, 0),
            (900, 640, 12),
            (900, 640, 24),
            (1350, 960, 36),
        ] {
            unsafe {
                let region = super::rounded_region(width, height, radius);
                assert!(!region.is_null());
                let mut bounds = std::mem::zeroed();
                assert!(GetRgnBox(region, &mut bounds) > 0);
                let actual = (bounds.left, bounds.top, bounds.right, bounds.bottom);
                let corners = [
                    (0, 0),
                    (width - 1, 0),
                    (0, height - 1),
                    (width - 1, height - 1),
                ]
                .map(|(x, y)| PtInRegion(region, x, y) != 0);
                let center = PtInRegion(region, width / 2, height / 2) != 0;
                DeleteObject(region);
                assert_eq!(actual, (0, 0, width, height), "radius={radius}");
                assert!(center);
                assert_eq!(corners, [radius < 8; 4]);
            }
        }
    }
}

pub fn setup(window: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        if let Err(error) = frame::attach(window) {
            crate::diagnostics::record("窗口", "玻璃边框", None, error);
        }
        let _ = window.set_theme(Some(tauri::Theme::Dark));
        after_show(window);
        // Neither System Backdrop nor Accent Blur clipped reliably on the user's
        // desktop. Keep the native surface transparent; draw the glass face in CSS.
        if let Err(error) = window_vibrancy::clear_acrylic(window) {
            crate::diagnostics::record("窗口", "清除矩形材质", None, error.to_string());
        }
        if let Err(error) = window_vibrancy::clear_blur(window) {
            crate::diagnostics::record("窗口", "清除 Accent Blur", None, error.to_string());
        }
        crate::diagnostics::record(
            "窗口",
            "玻璃材质",
            None,
            "原生背景透明；已关闭 System Backdrop 和 Accent Blur，使用 CSS 玻璃表面",
        );
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
            if let Err(error) = verify_clip(&target) {
                crate::diagnostics::record("窗口", "圆角核验", None, error);
            }
        });
    }
}
