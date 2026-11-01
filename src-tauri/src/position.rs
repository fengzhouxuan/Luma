//! 保存用户主动移动的位置；自动回退不修改偏好，重连只在唤醒时恢复。
use serde::{Deserialize, Serialize};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::Manager;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedPosition {
    pub monitor_id: String,
    pub monitor_name: String,
    pub scale_factor: f64,
    pub relative_x: f64,
    pub relative_y: f64,
}
impl SavedPosition {
    fn valid(&self) -> bool {
        !self.monitor_id.is_empty()
            && self.scale_factor.is_finite()
            && self.scale_factor > 0.0
            && self.relative_x.is_finite()
            && self.relative_y.is_finite()
            && (0.0..=1.0).contains(&self.relative_x)
            && (0.0..=1.0).contains(&self.relative_y)
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
    fn contains(self, other: Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x as i64 + other.width as i64 <= self.x as i64 + self.width as i64
            && other.y as i64 + other.height as i64 <= self.y as i64 + self.height as i64
    }
    fn overlap(self, other: Self) -> u64 {
        let width = (self.x as i64 + self.width as i64).min(other.x as i64 + other.width as i64)
            - (self.x as i64).max(other.x as i64);
        let height = (self.y as i64 + self.height as i64).min(other.y as i64 + other.height as i64)
            - (self.y as i64).max(other.y as i64);
        width.max(0) as u64 * height.max(0) as u64
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Display {
    pub id: String,
    pub name: String,
    pub area: Rect,
    pub scale: f64,
    pub primary: bool,
}
#[derive(Clone, Debug, PartialEq)]
struct Plan {
    display_id: String,
    rect: Rect,
    fallback: bool,
}

fn primary(displays: &[Display]) -> Option<&Display> {
    displays
        .iter()
        .find(|display| display.primary)
        .or_else(|| displays.first())
}

fn plan(
    display: &Display,
    relative_x: f64,
    relative_y: f64,
    logical_size: (f64, f64),
    fallback: bool,
) -> Plan {
    let width =
        ((logical_size.0 * display.scale).round() as u32).clamp(1, display.area.width.max(1));
    let height =
        ((logical_size.1 * display.scale).round() as u32).clamp(1, display.area.height.max(1));
    let offset_x = (relative_x * display.area.width.saturating_sub(width) as f64).round() as i64;
    let offset_y = (relative_y * display.area.height.saturating_sub(height) as f64).round() as i64;
    Plan {
        display_id: display.id.clone(),
        rect: Rect {
            x: (display.area.x as i64 + offset_x).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            y: (display.area.y as i64 + offset_y).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            width,
            height,
        },
        fallback,
    }
}

fn restore(
    preferred: Option<&SavedPosition>,
    displays: &[Display],
    size: (f64, f64),
) -> Option<Plan> {
    if let Some(preferred) = preferred.filter(|value| value.valid()) {
        if let Some(display) = displays
            .iter()
            .find(|display| display.id == preferred.monitor_id)
        {
            return Some(plan(
                display,
                preferred.relative_x,
                preferred.relative_y,
                size,
                false,
            ));
        }
    }
    Some(plan(
        primary(displays)?,
        0.5,
        0.5,
        size,
        preferred.is_some(),
    ))
}

fn capture(rect: Rect, displays: &[Display]) -> Option<SavedPosition> {
    let display = displays
        .iter()
        .max_by_key(|display| display.area.overlap(rect))?;
    if display.area.overlap(rect) == 0 {
        return None;
    }
    let relative = |position: i32, origin: i32, area: u32, window: u32| {
        let room = area.saturating_sub(window);
        if room == 0 {
            0.5
        } else {
            ((position as f64 - origin as f64) / room as f64).clamp(0.0, 1.0)
        }
    };
    Some(SavedPosition {
        monitor_id: display.id.clone(),
        monitor_name: display.name.clone(),
        scale_factor: display.scale,
        relative_x: relative(rect.x, display.area.x, display.area.width, rect.width),
        relative_y: relative(rect.y, display.area.y, display.area.height, rect.height),
    })
}

struct Engine {
    preferred: Option<SavedPosition>,
    fallback: bool,
    active_display: Option<String>,
}
impl Engine {
    fn wake(&mut self, displays: &[Display], size: (f64, f64)) -> Option<Plan> {
        let result = restore(self.preferred.as_ref(), displays, size)?;
        self.fallback = result.fallback;
        self.active_display = Some(result.display_id.clone());
        Some(result)
    }
    fn user_moved(&mut self, rect: Rect, displays: &[Display]) -> Option<SavedPosition> {
        let saved = capture(rect, displays)?;
        self.active_display = Some(saved.monitor_id.clone());
        self.preferred = Some(saved.clone());
        self.fallback = false;
        Some(saved)
    }
    fn topology_changed(
        &mut self,
        displays: &[Display],
        rect: Rect,
        size: (f64, f64),
    ) -> Option<Plan> {
        if !self.fallback {
            return self.wake(displays, size);
        }
        // 偏好屏重连也不立即跳屏；当前回退屏消失时才再次选择主屏。
        if self
            .active_display
            .as_ref()
            .is_some_and(|id| !displays.iter().any(|display| &display.id == id))
        {
            let result = plan(primary(displays)?, 0.5, 0.5, size, true);
            self.active_display = Some(result.display_id.clone());
            return Some(result);
        }
        if displays.iter().any(|display| display.area.contains(rect)) {
            return None;
        }
        let current = capture(rect, displays);
        let display = current
            .as_ref()
            .and_then(|saved| {
                displays
                    .iter()
                    .find(|display| display.id == saved.monitor_id)
            })
            .or_else(|| primary(displays))?;
        Some(plan(
            display,
            current.as_ref().map_or(0.5, |saved| saved.relative_x),
            current.as_ref().map_or(0.5, |saved| saved.relative_y),
            size,
            true,
        ))
    }
}

struct State {
    engine: Engine,
    displays: Vec<Display>,
    logical_size: (f64, f64),
}
struct PositionManager(Mutex<State>);
static TICK_PENDING: AtomicBool = AtomicBool::new(false);

fn inventory(window: &tauri::WebviewWindow) -> Result<Vec<Display>, String> {
    let primary_name = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .and_then(|monitor| monitor.name().cloned());
    let mut displays: Vec<_> = window
        .available_monitors()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|monitor| {
            let area = monitor.work_area();
            if area.size.width == 0
                || area.size.height == 0
                || !monitor.scale_factor().is_finite()
                || monitor.scale_factor() <= 0.0
            {
                return None;
            }
            let gdi_name = monitor.name().cloned().unwrap_or_default();
            let (id, name) = native::identity(&gdi_name);
            Some(Display {
                id,
                name,
                area: Rect {
                    x: area.position.x,
                    y: area.position.y,
                    width: area.size.width,
                    height: area.size.height,
                },
                scale: monitor.scale_factor(),
                primary: primary_name.as_ref() == Some(&gdi_name),
            })
        })
        .collect();
    displays.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(displays)
}

fn rect(window: &tauri::WebviewWindow) -> Result<Rect, String> {
    let point = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    Ok(Rect {
        x: point.x,
        y: point.y,
        width: size.width,
        height: size.height,
    })
}

fn apply(window: &tauri::WebviewWindow, result: &Plan) -> Result<(), String> {
    if rect(window)? == result.rect {
        return Ok(());
    }
    let point = tauri::PhysicalPosition::new(result.rect.x, result.rect.y);
    window.set_position(point).map_err(|e| e.to_string())?;
    // 跨 DPI 屏幕时先让 Windows 切换缩放，再明确限制窗口物理尺寸和位置。
    window
        .set_size(tauri::PhysicalSize::new(
            result.rect.width,
            result.rect.height,
        ))
        .map_err(|e| e.to_string())?;
    window.set_position(point).map_err(|e| e.to_string())?;
    crate::diagnostics::record(
        "窗口",
        "位置恢复",
        None,
        format!(
            "{}：({}, {})，{}×{}",
            if result.fallback {
                "临时回退"
            } else {
                "恢复偏好位置"
            },
            result.rect.x,
            result.rect.y,
            result.rect.width,
            result.rect.height
        ),
    );
    Ok(())
}

fn reconcile(window: &tauri::WebviewWindow, wake: bool) -> Result<(), String> {
    if native::dragging() {
        return Ok(());
    }
    let Some(manager) = window.app_handle().try_state::<PositionManager>() else {
        return Ok(());
    };
    let displays = inventory(window)?;
    if displays.is_empty() {
        return Ok(());
    }
    let mut state = manager
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let changed = state.displays != displays;
    let moved = native::take_user_move();
    let dirty = native::take_dirty();
    if moved && !changed {
        if let Some(saved) = state.engine.user_moved(rect(window)?, &displays) {
            crate::config::update(|config| config.window_position = Some(saved.clone()))?;
            crate::diagnostics::record(
                "窗口",
                "位置记忆",
                None,
                format!("已保存 {} 上的主动移动位置", saved.monitor_name),
            );
        }
    }
    state.displays = displays;
    let State {
        engine,
        displays,
        logical_size,
    } = &mut *state;
    let result = if wake {
        engine.wake(displays, *logical_size)
    } else if changed || dirty {
        engine.topology_changed(displays, rect(window)?, *logical_size)
    } else {
        None
    };
    if let Some(result) = result {
        apply(window, &result)?;
    }
    Ok(())
}

pub fn before_show(window: &tauri::WebviewWindow) {
    if let Err(error) = reconcile(window, true) {
        crate::diagnostics::record("窗口", "位置恢复", None, error);
    }
}

pub fn flush(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = reconcile(&window, false) {
            crate::diagnostics::record("窗口", "位置保存", None, error);
        }
    }
}

pub fn setup(app: &tauri::App) -> Result<(), String> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main");
    let logical_size = config.map_or((900.0, 640.0), |window| (window.width, window.height));
    app.manage(PositionManager(Mutex::new(State {
        engine: Engine {
            preferred: crate::config::get().window_position,
            fallback: false,
            active_display: None,
        },
        displays: inventory(&window)?,
        logical_size,
    })));
    native::attach(&window)?;
    before_show(&window);
    std::thread::Builder::new()
        .name("luma-display-watcher".into())
        .spawn(move || {
            let mut ticks = 0u8;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if native::closed() {
                    break;
                }
                ticks = (ticks + 1) % 4;
                if ticks != 0 && !native::pending() {
                    continue;
                }
                if TICK_PENDING.swap(true, Ordering::AcqRel) {
                    continue;
                }
                let target = window.clone();
                if window
                    .run_on_main_thread(move || {
                        if let Err(error) = reconcile(&target, false) {
                            crate::diagnostics::record("窗口", "显示器变化", None, error);
                        }
                        TICK_PENDING.store(false, Ordering::Release);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::sync::atomic::{AtomicI32, AtomicU64};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{DISPLAY_DEVICEW, EnumDisplayDevicesW},
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                EDD_GET_DEVICE_INTERFACE_NAME, GetWindowRect, WM_DISPLAYCHANGE, WM_DPICHANGED,
                WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_NCDESTROY, WM_SETTINGCHANGE,
            },
        },
    };
    const SUBCLASS_ID: usize = 0x4c554d50;
    static DRAGGING: AtomicBool = AtomicBool::new(false);
    static USER_MOVED: AtomicBool = AtomicBool::new(false);
    static DIRTY: AtomicBool = AtomicBool::new(false);
    static CLOSED: AtomicBool = AtomicBool::new(false);
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    static START_GENERATION: AtomicU64 = AtomicU64::new(0);
    static START_X: AtomicI32 = AtomicI32::new(0);
    static START_Y: AtomicI32 = AtomicI32::new(0);

    pub fn identity(name: &str) -> (String, String) {
        let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        let mut device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
        device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        if unsafe {
            EnumDisplayDevicesW(wide.as_ptr(), 0, &mut device, EDD_GET_DEVICE_INTERFACE_NAME)
        } != 0
        {
            let length = device
                .DeviceID
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(device.DeviceID.len());
            let id = String::from_utf16_lossy(&device.DeviceID[..length]).to_ascii_lowercase();
            if !id.is_empty() {
                let model = id.split('#').nth(1).unwrap_or(name).to_ascii_uppercase();
                return (id, format!("{model} ({name})"));
            }
        }
        (format!("gdi:{}", name.to_ascii_lowercase()), name.into())
    }
    pub fn attach(window: &tauri::WebviewWindow) -> Result<(), String> {
        let hwnd = window.hwnd().map_err(|e| e.to_string())?;
        if unsafe { SetWindowSubclass(hwnd.0 as _, Some(window_proc), SUBCLASS_ID, 0) } == 0 {
            return Err("无法安装窗口位置监听".into());
        }
        Ok(())
    }
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        match message {
            WM_ENTERSIZEMOVE => {
                let mut rect: RECT = unsafe { std::mem::zeroed() };
                unsafe {
                    GetWindowRect(hwnd, &mut rect);
                }
                START_X.store(rect.left, Ordering::Relaxed);
                START_Y.store(rect.top, Ordering::Relaxed);
                START_GENERATION.store(GENERATION.load(Ordering::Acquire), Ordering::Release);
                DRAGGING.store(true, Ordering::Release);
            }
            WM_EXITSIZEMOVE => {
                let mut rect: RECT = unsafe { std::mem::zeroed() };
                let valid = unsafe { GetWindowRect(hwnd, &mut rect) } != 0;
                if valid
                    && (rect.left != START_X.load(Ordering::Relaxed)
                        || rect.top != START_Y.load(Ordering::Relaxed))
                    && GENERATION.load(Ordering::Acquire)
                        == START_GENERATION.load(Ordering::Acquire)
                {
                    USER_MOVED.store(true, Ordering::Release);
                }
                DRAGGING.store(false, Ordering::Release);
            }
            WM_DISPLAYCHANGE => {
                GENERATION.fetch_add(1, Ordering::AcqRel);
                DIRTY.store(true, Ordering::Release);
            }
            WM_DPICHANGED | WM_SETTINGCHANGE => DIRTY.store(true, Ordering::Release),
            WM_NCDESTROY => {
                CLOSED.store(true, Ordering::Release);
                unsafe {
                    RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID);
                }
            }
            _ => {}
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }
    pub fn dragging() -> bool {
        DRAGGING.load(Ordering::Acquire)
    }
    pub fn take_user_move() -> bool {
        USER_MOVED.swap(false, Ordering::AcqRel)
    }
    pub fn take_dirty() -> bool {
        DIRTY.swap(false, Ordering::AcqRel)
    }
    pub fn pending() -> bool {
        USER_MOVED.load(Ordering::Acquire) || DIRTY.load(Ordering::Acquire)
    }
    pub fn closed() -> bool {
        CLOSED.load(Ordering::Acquire)
    }
}

#[cfg(not(windows))]
mod native {
    pub fn identity(name: &str) -> (String, String) {
        (name.into(), name.into())
    }
    pub fn attach(_: &tauri::WebviewWindow) -> Result<(), String> {
        Ok(())
    }
    pub fn dragging() -> bool {
        false
    }
    pub fn take_user_move() -> bool {
        false
    }
    pub fn take_dirty() -> bool {
        false
    }
    pub fn pending() -> bool {
        false
    }
    pub fn closed() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn displays() -> Vec<Display> {
        vec![
            Display {
                id: "aoc".into(),
                name: "AOC".into(),
                area: Rect {
                    x: -1920,
                    y: 0,
                    width: 1920,
                    height: 1032,
                },
                scale: 1.0,
                primary: false,
            },
            Display {
                id: "main".into(),
                name: "Main".into(),
                area: Rect {
                    x: 0,
                    y: 0,
                    width: 2560,
                    height: 1392,
                },
                scale: 1.0,
                primary: true,
            },
        ]
    }
    fn saved() -> SavedPosition {
        capture(
            Rect {
                x: -1379,
                y: 228,
                width: 900,
                height: 640,
            },
            &displays(),
        )
        .unwrap()
    }
    fn engine() -> Engine {
        Engine {
            preferred: Some(saved()),
            fallback: false,
            active_display: Some("aoc".into()),
        }
    }
    #[test]
    fn restores_negative_coordinates_by_identity_after_renumbering() {
        let mut monitors = displays();
        monitors[0].name = "Different GDI number".into();
        monitors.reverse();
        let result = restore(Some(&saved()), &monitors, (900.0, 640.0)).unwrap();
        assert_eq!(
            result.rect,
            Rect {
                x: -1379,
                y: 228,
                width: 900,
                height: 640
            }
        );
        assert!(!result.fallback);
    }
    #[test]
    fn disconnect_falls_back_without_overwriting_and_reconnect_waits_for_wake() {
        let mut state = engine();
        let original = state.preferred.clone();
        let all = displays();
        let remaining = vec![all[1].clone()];
        let fallback = state
            .topology_changed(
                &remaining,
                Rect {
                    x: -1379,
                    y: 228,
                    width: 900,
                    height: 640,
                },
                (900.0, 640.0),
            )
            .unwrap();
        assert_eq!(fallback.display_id, "main");
        assert!(remaining[0].area.contains(fallback.rect));
        assert_eq!(state.preferred, original);
        assert!(
            state
                .topology_changed(&all, fallback.rect, (900.0, 640.0))
                .is_none()
        );
        assert_eq!(
            state.wake(&all, (900.0, 640.0)).unwrap().rect,
            restore(original.as_ref(), &all, (900.0, 640.0))
                .unwrap()
                .rect
        );
    }
    #[test]
    fn dragging_after_fallback_replaces_preference_and_does_not_jump_back() {
        let mut state = engine();
        let all = displays();
        let remaining = vec![all[1].clone()];
        state.wake(&remaining, (900.0, 640.0));
        let moved = Rect {
            x: 300,
            y: 200,
            width: 900,
            height: 640,
        };
        state.user_moved(moved, &remaining).unwrap();
        assert_eq!(state.wake(&all, (900.0, 640.0)).unwrap().rect, moved);
    }
    #[test]
    fn dpi_resolution_and_taskbar_changes_keep_entire_window_visible() {
        let mut all = displays();
        all[0].area = Rect {
            x: -1280,
            y: 80,
            width: 1280,
            height: 920,
        };
        all[0].scale = 1.5;
        let result = restore(Some(&saved()), &all, (900.0, 640.0)).unwrap();
        assert!(all[0].area.contains(result.rect));
        assert_eq!(result.rect.width, 1280);
        assert_eq!(result.rect.height, 920);
    }
    #[test]
    fn empty_topology_and_invalid_saved_values_are_safe() {
        assert!(restore(Some(&saved()), &[], (900.0, 640.0)).is_none());
        let mut bad = saved();
        bad.relative_x = f64::NAN;
        let all = displays();
        let result = restore(Some(&bad), &all, (900.0, 640.0)).unwrap();
        assert_eq!(result.display_id, "main");
        assert!(all[1].area.contains(result.rect));
    }
}
