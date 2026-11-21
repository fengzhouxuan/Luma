use crate::position::Rect;
use image::{DynamicImage, ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tauri::{
    AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, WindowEvent,
    ipc::{InvokeBody, Request, Response},
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

pub(crate) const MAX_PIXELS: u64 = 40_000_000;
const MAX_PINS: usize = 8;
const MAX_IMAGE_BYTES: usize = 160_000_000;
static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct DisplayPlacement {
    id: String,
    rect: Rect,
    scale: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct SavedRegion {
    area: Rect,
    displays: Vec<DisplayPlacement>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct Preferences {
    #[serde(default)]
    last_region: Option<SavedRegion>,
    #[serde(default)]
    directory: Option<PathBuf>,
}

impl Preferences {
    pub(crate) fn is_empty(&self) -> bool {
        self.last_region.is_none() && self.directory.is_none()
    }
}

#[derive(Clone, Serialize)]
pub struct Scene {
    bounds: Rect,
    monitors: Vec<Rect>,
    windows: Vec<Rect>,
    cursor: Option<(i32, i32)>,
    last_area: Option<Rect>,
    reuse_last: bool,
}

struct Capture {
    scene: Scene,
    png: Vec<u8>,
    displays: Vec<DisplayPlacement>,
}

struct Session {
    label: String,
    capture: Option<Arc<Capture>>,
    ready: bool,
    restore_launcher: bool,
}

type Pins = HashMap<String, Arc<Vec<u8>>>;
fn session() -> &'static Mutex<Option<Session>> {
    static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();
    SESSION.get_or_init(Default::default)
}
fn pins() -> &'static Mutex<Pins> {
    static PINS: OnceLock<Mutex<Pins>> = OnceLock::new();
    PINS.get_or_init(Default::default)
}
pub fn active() -> bool {
    session().lock().unwrap().is_some()
}

fn desktop_bounds(monitors: &[Rect]) -> Result<Rect, String> {
    let min_x = monitors.iter().map(|r| r.x).min().ok_or("未找到显示器")?;
    let min_y = monitors.iter().map(|r| r.y).min().ok_or("未找到显示器")?;
    let max_x = monitors
        .iter()
        .map(|r| i64::from(r.x) + i64::from(r.width))
        .max()
        .unwrap();
    let max_y = monitors
        .iter()
        .map(|r| i64::from(r.y) + i64::from(r.height))
        .max()
        .unwrap();
    let width = max_x - i64::from(min_x);
    let height = max_y - i64::from(min_y);
    if width <= 0
        || height <= 0
        || width > 16384
        || height > 16384
        || width * height > MAX_PIXELS as i64
    {
        return Err("显示器组合超出截图尺寸上限".into());
    }
    Ok(Rect {
        x: min_x,
        y: min_y,
        width: width as u32,
        height: height as u32,
    })
}

fn remember_region(
    area: Rect,
    bounds: Rect,
    monitors: &[Rect],
    displays: &[DisplayPlacement],
) -> Option<SavedRegion> {
    if area.width < 2 || area.height < 2 || !bounds.contains(area) {
        return None;
    }
    let affected = monitors.iter().filter(|rect| rect.overlap(area) > 0);
    let displays = affected
        .map(|rect| {
            displays
                .iter()
                .find(|display| display.rect == *rect)
                .cloned()
        })
        .collect::<Option<Vec<_>>>()?;
    (!displays.is_empty()).then_some(SavedRegion { area, displays })
}

fn restore_region(
    saved: &SavedRegion,
    bounds: Rect,
    monitors: &[Rect],
    displays: &[DisplayPlacement],
) -> Option<Rect> {
    let current = remember_region(saved.area, bounds, monitors, displays)?;
    (current.displays.len() == saved.displays.len()
        && current
            .displays
            .iter()
            .all(|display| saved.displays.contains(display)))
    .then_some(saved.area)
}

pub(crate) fn encode(image: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>, String> {
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, format)
        .map_err(|e| e.to_string())?;
    Ok(bytes.into_inner())
}

#[cfg(windows)]
fn capture_desktop(reuse_last: bool) -> Result<Capture, String> {
    use xcap::Monitor;
    let monitors = Monitor::all().map_err(|e| e.to_string())?;
    let rectangles: Vec<Rect> = monitors
        .iter()
        .map(|monitor| {
            Ok(Rect {
                x: monitor.x()?,
                y: monitor.y()?,
                width: monitor.width()?,
                height: monitor.height()?,
            })
        })
        .collect::<Result<_, xcap::XCapError>>()
        .map_err(|e| e.to_string())?;
    let bounds = desktop_bounds(&rectangles)?;
    let displays: Vec<_> = monitors
        .iter()
        .zip(&rectangles)
        .filter_map(|(monitor, &rect)| {
            let scale = monitor.scale_factor().ok()?;
            (scale.is_finite() && scale > 0.0).then_some(DisplayPlacement {
                id: crate::position::display_id(&monitor.name().ok()?),
                rect,
                scale,
            })
        })
        .collect();
    let last_area = crate::config::get()
        .screenshot_prefs
        .last_region
        .and_then(|saved| restore_region(&saved, bounds, &rectangles, &displays));
    let windows = visible_windows();
    let cursor = {
        use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
        let mut point = POINT { x: 0, y: 0 };
        (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
    };
    let mut desktop =
        RgbaImage::from_pixel(bounds.width, bounds.height, image::Rgba([18, 22, 30, 255]));
    for (monitor, rect) in monitors.into_iter().zip(&rectangles) {
        let frame = monitor.capture_image().map_err(|e| e.to_string())?;
        if frame.dimensions() != (rect.width, rect.height) {
            return Err("截图期间显示器尺寸发生变化，请重试".into());
        }
        image::imageops::replace(
            &mut desktop,
            &frame,
            i64::from(rect.x) - i64::from(bounds.x),
            i64::from(rect.y) - i64::from(bounds.y),
        );
    }
    Ok(Capture {
        png: encode(&DynamicImage::ImageRgba8(desktop), ImageFormat::Png)?,
        scene: Scene {
            bounds,
            monitors: rectangles,
            windows,
            cursor,
            last_area,
            reuse_last,
        },
        displays,
    })
}

#[cfg(windows)]
fn visible_windows() -> Vec<Rect> {
    use windows_sys::Win32::{
        Foundation::{BOOL, HWND, LPARAM, RECT},
        Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
        UI::WindowsAndMessaging::{
            EnumWindows, GWL_EXSTYLE, GetShellWindow, GetWindowLongPtrW, GetWindowRect,
            GetWindowTextLengthW, IsIconic, IsWindowVisible, WS_EX_TRANSPARENT,
        },
    };
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        unsafe {
            if hwnd == GetShellWindow()
                || IsWindowVisible(hwnd) == 0
                || IsIconic(hwnd) != 0
                || GetWindowTextLengthW(hwnd) == 0
                || (GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT) != 0
            {
                return 1;
            }
            let mut cloaked = 0u32;
            if DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED as u32,
                (&mut cloaked as *mut u32).cast(),
                4,
            ) >= 0
                && cloaked != 0
            {
                return 1;
            }
            let mut rect: RECT = std::mem::zeroed();
            if DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                (&mut rect as *mut RECT).cast(),
                std::mem::size_of::<RECT>() as u32,
            ) < 0
                && GetWindowRect(hwnd, &mut rect) == 0
            {
                return 1;
            }
            let width = rect.right.saturating_sub(rect.left);
            let height = rect.bottom.saturating_sub(rect.top);
            if width > 10 && height > 10 {
                let windows = &mut *(data as *mut Vec<Rect>);
                windows.push(Rect {
                    x: rect.left,
                    y: rect.top,
                    width: width as u32,
                    height: height as u32,
                });
            }
            1
        }
    }
    let mut windows = Vec::new();
    unsafe {
        EnumWindows(Some(visit), (&mut windows as *mut Vec<Rect>) as LPARAM);
    }
    windows
}

#[cfg(not(windows))]
fn capture_desktop(_reuse_last: bool) -> Result<Capture, String> {
    Err("截图仅支持 Windows".into())
}

pub(crate) fn close_session(app: &AppHandle, label: &str) {
    let mut state = session().lock().unwrap();
    if state.as_ref().is_some_and(|s| s.label == label) {
        *state = None;
    }
    drop(state);
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.destroy();
    }
}

fn fail(app: &AppHandle, label: &str, error: String) {
    let restore = session()
        .lock()
        .unwrap()
        .as_ref()
        .filter(|s| s.label == label)
        .map(|s| s.restore_launcher);
    let Some(restore) = restore else {
        return;
    };
    close_session(app, label);
    crate::diagnostics::record("截图", "桌面采集", None, &error);
    if restore {
        crate::runtime::show(app);
    }
    let _ = app.emit_to("main", "screenshot-error", error);
}

fn build_overlay(app: &AppHandle, label: &str, capture: Arc<Capture>) -> Result<(), String> {
    let bounds = capture.scene.bounds;
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title("Luma 截图")
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    window
        .set_position(PhysicalPosition::new(bounds.x, bounds.y))
        .map_err(|e| e.to_string())?;
    window
        .set_size(PhysicalSize::new(bounds.width, bounds.height))
        .map_err(|e| e.to_string())?;
    let label = label.to_string();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            let mut state = session().lock().unwrap();
            if state.as_ref().is_some_and(|s| s.label == label) {
                *state = None;
            }
        }
    });
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        crate::disable_window_corner_and_shadow(hwnd.0 as isize);
    }
    Ok(())
}

pub fn start(app: &AppHandle, reuse_last: bool) -> Result<(), String> {
    if crate::runtime::native_dialog_open() {
        return Err("请先关闭当前对话框".into());
    }
    let mut state = session().lock().unwrap();
    if state.is_some() {
        return Ok(());
    }
    let label = format!("screenshot-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let main = app.get_webview_window("main").ok_or("Luma 主窗口不可用")?;
    let restore_launcher = main.is_visible().unwrap_or(false);
    *state = Some(Session {
        label: label.clone(),
        capture: None,
        ready: false,
        restore_launcher,
    });
    drop(state);
    dismiss_copy_toasts(app);
    if let Err(e) = main.hide() {
        close_session(app, &label);
        return Err(e.to_string());
    }
    #[cfg(windows)]
    crate::double_alt::WINDOW_VISIBLE.store(false, Ordering::SeqCst);
    let app = app.clone();
    let watchdog = app.clone();
    let pending_label = label.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(15));
        let handle = watchdog.clone();
        let _ = watchdog.run_on_main_thread(move || {
            let pending = session()
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|s| s.label == pending_label && !s.ready);
            if pending {
                fail(&handle, &pending_label, "截图界面加载超时，请重试".into());
            }
        });
    });
    std::thread::spawn(move || {
        // Wait for DWM to compose the desktop after hiding the launcher and tray menu.
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Graphics::Dwm::DwmFlush();
        }
        std::thread::sleep(Duration::from_millis(90));
        let result = capture_desktop(reuse_last).map(Arc::new);
        let handle = app.clone();
        let job_label = label.clone();
        let scheduled = app.run_on_main_thread(move || match result {
            Ok(capture) => {
                {
                    let mut state = session().lock().unwrap();
                    let Some(s) = state.as_mut().filter(|s| s.label == job_label) else {
                        return;
                    };
                    s.capture = Some(capture.clone());
                }
                if let Err(error) = build_overlay(&handle, &job_label, capture) {
                    fail(&handle, &job_label, error);
                }
            }
            Err(error) => fail(&handle, &job_label, error),
        });
        if scheduled.is_err() {
            close_session(&app, &label);
        }
    });
    Ok(())
}

pub fn start_from_shortcut(app: &AppHandle, reuse_last: bool) {
    if let Err(error) = start(app, reuse_last) {
        let _ = app.emit_to("main", "screenshot-error", error);
    }
}

#[tauri::command]
pub fn start_screenshot(window: WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("无效截图入口".into());
    }
    start(window.app_handle(), false)
}

fn current(window: &WebviewWindow) -> Result<Arc<Capture>, String> {
    session()
        .lock()
        .unwrap()
        .as_ref()
        .filter(|s| s.label == window.label())
        .and_then(|s| s.capture.clone())
        .ok_or_else(|| "截图会话已结束".into())
}

pub(crate) fn long_selection(window: &WebviewWindow, area: Rect) -> Result<Rect, String> {
    let capture = current(window)?;
    capture
        .scene
        .monitors
        .iter()
        .copied()
        .find(|screen| screen.contains(area))
        .ok_or_else(|| "长截图选区须位于同一块显示器内".into())
}

#[tauri::command]
pub fn screenshot_scene(window: WebviewWindow) -> Result<Scene, String> {
    Ok(current(&window)?.scene.clone())
}

#[tauri::command]
pub fn screenshot_pixels(window: WebviewWindow) -> Result<Response, String> {
    if let Some(png) = pins().lock().unwrap().get(window.label()).cloned() {
        return Ok(Response::new(png.as_ref().clone()));
    }
    Ok(Response::new(current(&window)?.png.clone()))
}

#[tauri::command]
pub fn screenshot_ready(window: WebviewWindow) -> Result<(), String> {
    current(&window)?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        crate::force_foreground(hwnd.0 as isize);
    }
    if let Some(s) = session()
        .lock()
        .unwrap()
        .as_mut()
        .filter(|s| s.label == window.label())
    {
        s.ready = true;
    }
    Ok(())
}

#[tauri::command]
pub fn screenshot_close(window: WebviewWindow, error: Option<String>) -> Result<(), String> {
    if window.label().starts_with("screenshot-pin-") {
        return window.destroy().map_err(|e| e.to_string());
    }
    current(&window)?;
    if let Some(error) = error {
        fail(window.app_handle(), window.label(), error);
    } else {
        close_session(window.app_handle(), window.label());
    }
    Ok(())
}

fn open_pin(
    app: &AppHandle,
    label: &str,
    png: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<(), String> {
    {
        let mut images = pins().lock().unwrap();
        if images.len() >= MAX_PINS {
            return Err("贴图已达上限，请先关闭一张".into());
        }
        images.insert(label.into(), Arc::new(png));
    }
    let result = (|| -> tauri::Result<()> {
        let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
            .title("Luma 贴图")
            .decorations(false)
            .shadow(false)
            .visible(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .build()?;
        let label = label.to_string();
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                pins().lock().unwrap().remove(&label);
            }
        });
        let cursor = app.cursor_position()?;
        let monitor = app
            .monitor_from_point(cursor.x, cursor.y)?
            .or(app.primary_monitor()?)
            .ok_or(tauri::Error::WindowNotFound)?;
        let work = monitor.work_area();
        let scale = (f64::from(work.size.width) * 0.78 / f64::from(width))
            .min(f64::from(work.size.height) * 0.78 / f64::from(height))
            .min(1.0);
        let w = (f64::from(width) * scale).round().max(1.0) as u32;
        let h = (f64::from(height) * scale).round().max(1.0) as u32;
        let x = work.position.x + (work.size.width.saturating_sub(w) / 2) as i32;
        let y = work.position.y + (work.size.height.saturating_sub(h) / 2) as i32;
        window.set_position(PhysicalPosition::new(x, y))?;
        window.set_size(PhysicalSize::new(w, h))?;
        window.show()?;
        Ok(())
    })();
    if result.is_err() {
        pins().lock().unwrap().remove(label);
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.destroy();
        }
    }
    result.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn screenshot_zoom_pin(window: WebviewWindow, zoom_in: bool) -> Result<(), String> {
    if !pins().lock().unwrap().contains_key(window.label()) {
        return Err("贴图已关闭".into());
    }
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("显示器不可用")?;
    let work = monitor.work_area();
    let factor: f64 = if zoom_in { 1.15 } else { 1.0 / 1.15 };
    let max = (f64::from(work.size.width) / f64::from(size.width))
        .min(f64::from(work.size.height) / f64::from(size.height));
    let min = (48.0 / f64::from(size.width.max(size.height))).min(max);
    let factor = factor.clamp(min, max);
    let width = (f64::from(size.width) * factor).round().max(1.0) as u32;
    let height = (f64::from(size.height) * factor).round().max(1.0) as u32;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let x = pos.x.clamp(
        work.position.x,
        work.position.x + work.size.width.saturating_sub(width) as i32,
    );
    let y = pos.y.clamp(
        work.position.y,
        work.position.y + work.size.height.saturating_sub(height) as i32,
    );
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    window
        .set_size(PhysicalSize::new(width, height))
        .map_err(|e| e.to_string())
}

pub(crate) fn output_monitor(window: &WebviewWindow, area: Option<Rect>) -> Option<Monitor> {
    if let Some(area) = area {
        let monitors = window.available_monitors().unwrap_or_default();
        let overlap = |monitor: &Monitor| {
            Rect {
                x: monitor.position().x,
                y: monitor.position().y,
                width: monitor.size().width,
                height: monitor.size().height,
            }
            .overlap(area)
        };
        if let Some(monitor) = monitors.into_iter().max_by_key(overlap)
            && overlap(&monitor) > 0
        {
            return Some(monitor);
        }
    }
    window.current_monitor().ok().flatten()
}

#[cfg(windows)]
fn save_dialog_owner(window: &WebviewWindow, monitor: &Monitor) -> Result<WebviewWindow, String> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let parent = window.clone();
    let work = *monitor.work_area();
    window
        .app_handle()
        .run_on_main_thread(move || {
            let result = (|| -> tauri::Result<WebviewWindow> {
                let label = format!("screenshot-save-{}", NEXT.fetch_add(1, Ordering::Relaxed));
                let anchor = WebviewWindowBuilder::new(
                    parent.app_handle(),
                    label,
                    WebviewUrl::App("index.html".into()),
                )
                .owner(&parent)?
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
                .visible(false)
                .inner_size(1.0, 1.0)
                .build()?;
                if let Err(error) = anchor.set_position(PhysicalPosition::new(
                    work.position.x + 32,
                    work.position.y + 32,
                )) {
                    let _ = anchor.destroy();
                    return Err(error);
                }
                Ok(anchor)
            })();
            let _ = send.send(result.map_err(|err| err.to_string()));
        })
        .map_err(|e| e.to_string())?;
    receive.recv().map_err(|e| e.to_string())?
}

fn choose_save_path(
    window: &WebviewWindow,
    monitor: Option<&Monitor>,
) -> Result<Option<PathBuf>, String> {
    static SAVE_DIALOG: Mutex<()> = Mutex::new(());
    let _dialog = SAVE_DIALOG
        .try_lock()
        .map_err(|_| "请先关闭当前保存对话框")?;
    let _hold = crate::runtime::hold_visible();
    let app = window.app_handle();
    let starting_directory = [
        crate::config::get().screenshot_prefs.directory,
        app.path().picture_dir().ok(),
        app.path().document_dir().ok(),
        app.path().home_dir().ok(),
    ]
    .into_iter()
    .flatten()
    .find(|path| path.is_absolute() && path.is_dir());
    #[cfg(windows)]
    let owner = monitor
        .map(|monitor| save_dialog_owner(window, monitor))
        .transpose()?;
    #[cfg(not(windows))]
    let owner: Option<WebviewWindow> = None;
    let stamp = crate::diagnostics::now();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("保存截图")
        .set_parent(owner.as_ref().unwrap_or(window))
        .add_filter("PNG", &["png"])
        .add_filter("JPEG", &["jpg", "jpeg"])
        .set_file_name(format!("Luma-{stamp}.png"));
    if let Some(directory) = starting_directory {
        dialog = dialog.set_directory(directory);
    }
    let file = dialog.blocking_save_file();
    if let Some(owner) = owner {
        let _ = owner.destroy();
        let _ = window.set_focus();
    }
    file.map(|file| file.into_path().map_err(|e| e.to_string()))
        .transpose()
}

fn destroy_after(window: WebviewWindow, duration: Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(duration);
        let _ = window.destroy();
    });
}

fn dismiss_copy_toasts(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("screenshot-toast-") {
            let _ = window.destroy();
        }
    }
}

pub(crate) fn show_copy_toast(app: &AppHandle, monitor: &Monitor) -> tauri::Result<()> {
    dismiss_copy_toasts(app);
    let label = format!("screenshot-toast-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let toast = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title("Luma 截图提示")
        .decorations(false)
        .shadow(false)
        .focused(false)
        .focusable(false)
        .visible(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .build()?;
    let work = monitor.work_area();
    let width = (180.0 * monitor.scale_factor()).round() as u32;
    let height = (42.0 * monitor.scale_factor()).round() as u32;
    let margin = (28.0 * monitor.scale_factor()).round() as u32;
    let x = work.position.x + (work.size.width.saturating_sub(width) / 2) as i32;
    let y = work.position.y + work.size.height.saturating_sub(height + margin) as i32;
    destroy_after(toast.clone(), Duration::from_secs(4));
    toast.set_position(PhysicalPosition::new(x, y))?;
    toast.set_size(PhysicalSize::new(width, height))?;
    toast.set_ignore_cursor_events(true)
}

#[tauri::command]
pub fn screenshot_toast_ready(window: WebviewWindow) -> Result<(), String> {
    if !window.label().starts_with("screenshot-toast-") || active() {
        return Err("截图提示已关闭".into());
    }
    window.show().map_err(|e| e.to_string())?;
    destroy_after(window, Duration::from_millis(1250));
    Ok(())
}

pub(crate) fn copy_image(app: &AppHandle, rgba: &RgbaImage) -> Result<(), String> {
    let screenshot = tauri::image::Image::new(rgba.as_raw(), rgba.width(), rgba.height());
    app.clipboard()
        .write_image(&screenshot)
        .map_err(|error| error.to_string())
}

pub(crate) fn save_image(
    window: &WebviewWindow,
    monitor: Option<&Monitor>,
    image: DynamicImage,
    original_png: Option<Vec<u8>>,
) -> Result<Option<PathBuf>, String> {
    let Some(path) = choose_save_path(window, monitor)? else {
        return Ok(None);
    };
    let jpeg = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg"));
    let bytes = if jpeg {
        encode(
            &DynamicImage::ImageRgb8(image.into_rgb8()),
            ImageFormat::Jpeg,
        )?
    } else if let Some(png) = original_png {
        png
    } else {
        encode(&image, ImageFormat::Png)?
    };
    std::fs::write(&path, bytes).map_err(|e| format!("保存截图失败：{e}"))?;
    Ok(Some(path))
}

pub(crate) fn remember_output_directory(path: &std::path::Path) {
    if let Some(directory) = path.parent()
        && let Err(error) = crate::config::update(|config| {
            config.screenshot_prefs.directory = Some(directory.to_path_buf());
        })
    {
        crate::diagnostics::record("截图", "保存截图偏好", None, &error);
    }
}

#[tauri::command]
pub async fn screenshot_output(
    window: WebviewWindow,
    request: Request<'_>,
) -> Result<bool, String> {
    let is_pin = pins().lock().unwrap().contains_key(window.label());
    let capture = if is_pin {
        None
    } else {
        Some(current(&window)?)
    };
    let area = if let Some(capture) = &capture {
        let header = request
            .headers()
            .get("x-luma-region")
            .ok_or("缺少选区位置")?;
        let area: Rect = serde_json::from_slice(header.as_bytes()).map_err(|_| "选区位置无效")?;
        if !capture.scene.bounds.contains(area)
            || !capture
                .scene
                .monitors
                .iter()
                .any(|monitor| monitor.overlap(area) > 0)
        {
            return Err("选区超出屏幕范围".into());
        }
        Some(area)
    } else {
        None
    };
    let action = request
        .headers()
        .get("x-luma-action")
        .and_then(|value| value.to_str().ok())
        .ok_or("缺少输出方式")?
        .to_string();
    let InvokeBody::Raw(data) = request.body() else {
        return Err("缺少截图数据".into());
    };
    if data.len() > MAX_IMAGE_BYTES {
        return Err("截图文件过大".into());
    }
    let data = data.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut reader = image::ImageReader::with_format(Cursor::new(&data), ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(MAX_PIXELS * 6);
        reader.limits(limits);
        let image = reader.decode().map_err(|e| format!("无法读取截图：{e}"))?;
        if u64::from(image.width()) * u64::from(image.height()) > MAX_PIXELS {
            return Err("截图尺寸超出上限".into());
        }
        if area.is_some_and(|area| (area.width, area.height) != (image.width(), image.height())) {
            return Err("截图尺寸与选区不一致".into());
        }
        let app = window.app_handle();
        let mut directory = None;
        match action.as_str() {
            "copy" => {
                copy_image(app, &image.into_rgba8())?;
            }
            "save" => {
                let monitor = area.and_then(|area| output_monitor(&window, Some(area)));
                let Some(path) = save_image(&window, monitor.as_ref(), image, Some(data))? else {
                    return Ok(false);
                };
                directory = path.parent().map(PathBuf::from);
            }
            "pin" if !is_pin => {
                let label = format!("screenshot-pin-{}", NEXT.fetch_add(1, Ordering::Relaxed));
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                let handle = app.clone();
                let (width, height) = (image.width(), image.height());
                app.run_on_main_thread(move || {
                    let _ = tx.send(open_pin(&handle, &label, data, width, height));
                })
                .map_err(|e| e.to_string())?;
                rx.recv().map_err(|e| e.to_string())??;
            }
            _ => return Err("不支持的截图输出方式".into()),
        }
        let last_region = capture.as_ref().zip(area).and_then(|(capture, area)| {
            remember_region(
                area,
                capture.scene.bounds,
                &capture.scene.monitors,
                &capture.displays,
            )
        });
        if (directory.is_some() || capture.is_some())
            && let Err(error) = crate::config::update(|config| {
                if capture.is_some() {
                    config.screenshot_prefs.last_region = last_region;
                }
                if directory.is_some() {
                    config.screenshot_prefs.directory = directory;
                }
            })
        {
            crate::diagnostics::record("截图", "保存截图偏好", None, &error);
        }
        let monitor = (action == "copy")
            .then(|| output_monitor(&window, area))
            .flatten();
        if !is_pin {
            let app = app.clone();
            let label = window.label().to_string();
            let handle = app.clone();
            app.run_on_main_thread(move || {
                close_session(&handle, &label);
                if let Some(monitor) = monitor {
                    let _ = show_copy_toast(&handle, &monitor);
                }
            })
            .map_err(|e| e.to_string())?;
        } else if let Some(monitor) = monitor {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                if !active() {
                    let _ = show_copy_toast(&handle, &monitor);
                }
            });
        }
        Ok(true)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_bounds_keep_negative_origins_and_physical_pixels() {
        let screens = [
            Rect {
                x: -1920,
                y: 100,
                width: 1920,
                height: 1080,
            },
            Rect {
                x: 0,
                y: -100,
                width: 2560,
                height: 1440,
            },
        ];
        assert_eq!(
            desktop_bounds(&screens).unwrap(),
            Rect {
                x: -1920,
                y: -100,
                width: 4480,
                height: 1440
            }
        );
        assert!(desktop_bounds(&[]).is_err());
        assert!(
            desktop_bounds(&[Rect {
                x: 0,
                y: 0,
                width: u32::MAX,
                height: 100
            }])
            .is_err()
        );
    }

    #[test]
    fn last_region_requires_the_same_displays_and_scale_underneath_it() {
        let aoc = Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let primary = Rect {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        };
        let area = Rect {
            x: -100,
            y: 150,
            width: 200,
            height: 300,
        };
        let bounds = desktop_bounds(&[aoc, primary]).unwrap();
        let aoc_display = DisplayPlacement {
            id: "aoc".into(),
            rect: aoc,
            scale: 1.0,
        };
        let primary_display = DisplayPlacement {
            id: "primary".into(),
            rect: primary,
            scale: 1.25,
        };
        let displays = [aoc_display.clone(), primary_display.clone()];
        let previous = remember_region(area, bounds, &[aoc, primary], &displays).unwrap();
        assert_eq!(
            restore_region(&previous, bounds, &[primary, aoc], &displays),
            Some(area)
        );
        assert_eq!(
            restore_region(
                &previous,
                primary,
                &[primary],
                std::slice::from_ref(&primary_display)
            ),
            None
        );
        let resized = DisplayPlacement {
            scale: 1.5,
            ..aoc_display.clone()
        };
        assert_eq!(
            restore_region(
                &previous,
                bounds,
                &[aoc, primary],
                &[resized, primary_display]
            ),
            None
        );
        let single_area = Rect { x: -400, ..area };
        let single = remember_region(single_area, bounds, &[aoc, primary], &displays).unwrap();
        assert_eq!(
            restore_region(&single, aoc, &[aoc], std::slice::from_ref(&aoc_display)),
            Some(single_area)
        );
        let replacement = DisplayPlacement {
            id: "replacement".into(),
            ..aoc_display
        };
        assert_eq!(restore_region(&single, aoc, &[aoc], &[replacement]), None);
    }
}
