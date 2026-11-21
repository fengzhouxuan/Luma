use crate::{
    position::Rect,
    screenshot,
    screenshot_stitch::{Stitcher, Update},
};
use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent, ipc::Response,
};

const CAPTURE: u8 = 0;
const FINISH: u8 = 1;
const CANCEL: u8 = 2;
const MAX_SIDE: u32 = 16_384;
const SAMPLE_INTERVAL: Duration = Duration::from_millis(105);
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
static LONG_SHOT: Mutex<Option<Arc<Job>>> = Mutex::new(None);

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Starting,
    Scrolling,
    Finishing,
    Preview,
    Failed,
}

#[derive(Clone, Serialize)]
pub struct Status {
    phase: Phase,
    width: u32,
    height: u32,
    frames: u32,
    note: Option<String>,
}

struct State {
    status: Status,
    image: Option<Arc<RgbaImage>>,
}

struct Job {
    label: String,
    editor: String,
    area: Rect,
    screen: Rect,
    request: AtomicU8,
    state: Mutex<State>,
    output: Mutex<()>,
}

fn current(window: &WebviewWindow) -> Result<Arc<Job>, String> {
    LONG_SHOT
        .lock()
        .unwrap()
        .as_ref()
        .filter(|job| job.label == window.label())
        .cloned()
        .ok_or_else(|| "长截图会话已结束".into())
}

fn close(app: &AppHandle, job: &Job) {
    let removed = LONG_SHOT
        .lock()
        .unwrap()
        .take_if(|active| active.label == job.label);
    if removed.is_none() {
        return;
    }
    if let Some(control) = app.get_webview_window(&job.label) {
        let _ = control.destroy();
    }
    screenshot::close_session(app, &job.editor);
}

fn work_area(monitor: &Monitor) -> Rect {
    let work = monitor.work_area();
    Rect {
        x: work.position.x,
        y: work.position.y,
        width: work.size.width,
        height: work.size.height,
    }
}

fn place_control(monitor: &Monitor, selection: Rect) -> Rect {
    let work = work_area(monitor);
    let physical = |size: f64| (size * monitor.scale_factor()).round() as u32;
    let width = physical(362.0).min(work.width);
    let height = physical(126.0).min(work.height);
    let margin = physical(10.0);
    let inset_x = margin.min(work.width.saturating_sub(width) / 2);
    let inset_y = margin.min(work.height.saturating_sub(height) / 2);
    let left = work.x + inset_x as i32;
    let right = work.x + (work.width - width - inset_x) as i32;
    let top = work.y + inset_y as i32;
    let bottom = work.y + (work.height - height - inset_y) as i32;
    [(right, top), (right, bottom), (left, top), (left, bottom)]
        .into_iter()
        .map(|(x, y)| Rect {
            x,
            y,
            width,
            height,
        })
        .min_by_key(|place| place.overlap(selection))
        .expect("control has four candidate positions")
}

#[cfg(windows)]
fn exclude_controls(window: &WebviewWindow) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowDisplayAffinity, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
    };
    let hwnd = window.hwnd().map_err(|error| error.to_string())?.0;
    let mut affinity = 0;
    if unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } != 0
        && unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) } != 0
        && affinity == WDA_EXCLUDEFROMCAPTURE
    {
        Ok(())
    } else {
        Err("请缩小选区，在显示器上留出长截图控制条的位置".into())
    }
}

#[cfg(windows)]
fn begin(editor: &WebviewWindow, area: Rect) -> Result<(), String> {
    use crate::screenshot_outline::Outline;
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOWNOACTIVATE, ShowWindow};

    if LONG_SHOT.lock().unwrap().is_some() {
        return Err("长截图已在运行".into());
    }
    if area.width < 96 || area.height < 120 {
        return Err("请框选至少 96 × 120 像素的滚动区域".into());
    }
    let screen = screenshot::long_selection(editor, area)?;
    let monitor = screenshot::output_monitor(editor, Some(area)).ok_or("显示器不可用")?;
    let app = editor.app_handle();
    let label = format!("screenshot-long-{}", editor.label());
    let place = place_control(&monitor, area);
    let control = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title("Luma 长截图")
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .resizable(false)
        .build()
        .map_err(|error| error.to_string())?;
    let show = (|| -> Result<Outline, String> {
        control
            .set_position(PhysicalPosition::new(place.x, place.y))
            .map_err(|error| error.to_string())?;
        control
            .set_size(PhysicalSize::new(place.width, place.height))
            .map_err(|error| error.to_string())?;
        if place.overlap(area) > 0 {
            exclude_controls(&control)?;
        }
        let hwnd = control.hwnd().map_err(|error| error.to_string())?.0;
        let editor_hwnd = editor.hwnd().map_err(|error| error.to_string())?.0;
        let disabled = 1i32;
        unsafe {
            DwmSetWindowAttribute(
                editor_hwnd,
                DWMWA_TRANSITIONS_FORCEDISABLED as u32,
                (&disabled as *const i32).cast(),
                std::mem::size_of_val(&disabled) as u32,
            );
        }
        let outline = Outline::show(area, hwnd)?;
        if let Err(error) = editor.hide() {
            outline.close();
            return Err(error.to_string());
        }
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        Ok(outline)
    })();
    let outline = match show {
        Ok(outline) => outline,
        Err(error) => {
            let _ = control.destroy();
            return Err(error);
        }
    };
    let job = Arc::new(Job {
        label,
        editor: editor.label().to_string(),
        area,
        screen,
        request: AtomicU8::new(CAPTURE),
        state: Mutex::new(State {
            status: Status {
                phase: Phase::Starting,
                width: area.width,
                height: 0,
                frames: 0,
                note: None,
            },
            image: None,
        }),
        output: Mutex::new(()),
    });
    *LONG_SHOT.lock().unwrap() = Some(job.clone());
    let window_app = app.clone();
    let window_job = job.clone();
    control.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            window_job.request.store(CANCEL, Ordering::Release);
            let phase = window_job.state.lock().unwrap().status.phase;
            if matches!(phase, Phase::Preview | Phase::Failed) {
                close(&window_app, &window_job);
            }
        }
    });
    let app = app.clone();
    std::thread::spawn(move || {
        let result = capture(&job);
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            outline.close();
            finish_capture(&handle, &job, result);
        });
    });
    Ok(())
}

#[tauri::command]
pub async fn screenshot_start_long(window: WebviewWindow, area: Rect) -> Result<(), String> {
    #[cfg(windows)]
    {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let app = window.app_handle().clone();
        app.run_on_main_thread(move || {
            let _ = send.send(begin(&window, area));
        })
        .map_err(|error| error.to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            receive.recv().map_err(|error| error.to_string())?
        })
        .await
        .map_err(|error| error.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = (window, area);
        Err("长截图仅支持 Windows".into())
    }
}

#[cfg(windows)]
fn monitor_rect(monitor: &xcap::Monitor) -> Option<Rect> {
    Some(Rect {
        x: monitor.x().ok()?,
        y: monitor.y().ok()?,
        width: monitor.width().ok()?,
        height: monitor.height().ok()?,
    })
}

#[cfg(windows)]
fn capture(job: &Job) -> Result<Option<RgbaImage>, String> {
    use crate::screenshot_region::RegionSampler;

    // Allow restored application windows to settle before taking the reference frame.
    std::thread::sleep(Duration::from_millis(200));
    unsafe { windows_sys::Win32::Graphics::Dwm::DwmFlush() };
    if job.request.load(Ordering::Acquire) == CANCEL {
        return Ok(None);
    }
    let monitor = xcap::Monitor::all()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|monitor| monitor_rect(monitor) == Some(job.screen))
        .ok_or("截图所在的显示器已变化")?;
    let area = job.area;
    let max_height = MAX_SIDE.min((screenshot::MAX_PIXELS / u64::from(area.width)) as u32);
    let mut sampler = RegionSampler::new(area)?;
    let mut stitch = Stitcher::new(sampler.sample()?, max_height);
    {
        let mut state = job.state.lock().unwrap();
        state.status.phase = Phase::Scrolling;
        state.status.height = area.height;
        state.status.frames = 1;
    }
    let mut last_progress = Instant::now();
    let mut last_display_check = Instant::now();
    let mut last_sample = Instant::now();
    while job.request.load(Ordering::Acquire) != CANCEL {
        std::thread::sleep(SAMPLE_INTERVAL.saturating_sub(last_sample.elapsed()));
        last_sample = Instant::now();
        if job.request.load(Ordering::Acquire) != CAPTURE {
            break;
        }
        if last_display_check.elapsed() >= Duration::from_secs(1) {
            last_display_check = Instant::now();
            if monitor_rect(&monitor) != Some(job.screen) {
                job.state.lock().unwrap().status.note = Some("显示器已变化，已结束拼接".into());
                break;
            }
        }
        let frame = match sampler.sample() {
            Ok(frame) => frame,
            Err(error) => {
                crate::diagnostics::record("截图", "读取长截图选区", None, error);
                job.state.lock().unwrap().status.note = Some("读取选区失败，已结束拼接".into());
                break;
            }
        };
        let update = stitch.append(frame);
        let mut state = job.state.lock().unwrap();
        match update {
            Update::Appended => {
                last_progress = Instant::now();
                state.status.height = stitch.height();
                state.status.frames += 1;
                state.status.note = None;
            }
            Update::Unchanged => state.status.note = None,
            Update::NoOverlap
                if state.status.phase == Phase::Scrolling
                    && last_progress.elapsed() >= Duration::from_millis(420) =>
            {
                state.status.note = Some("当前画面接不上，请稍向上回滚，再分段向下滚动".into());
            }
            Update::NoOverlap => {}
            Update::Limit => {
                state.status.note = Some("已达到长图尺寸上限".into());
                break;
            }
        }
        if job.request.load(Ordering::Acquire) == FINISH {
            break;
        }
        if last_progress.elapsed() >= IDLE_TIMEOUT {
            state.status.note = Some("一段时间未拼接新内容，已结束采集".into());
            break;
        }
    }
    if job.request.load(Ordering::Acquire) == CANCEL {
        return Ok(None);
    }
    Ok(Some(stitch.into_image()))
}

#[cfg(windows)]
fn finish_capture(app: &AppHandle, job: &Job, result: Result<Option<RgbaImage>, String>) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_NONE};

    let Some(control) = app.get_webview_window(&job.label) else {
        close(app, job);
        return;
    };
    if job.request.load(Ordering::Acquire) == CANCEL {
        close(app, job);
        return;
    }
    if let Ok(hwnd) = control.hwnd() {
        unsafe { SetWindowDisplayAffinity(hwnd.0, WDA_NONE) };
    }
    let mut state = job.state.lock().unwrap();
    match result {
        Ok(Some(image)) => {
            state.status.phase = Phase::Preview;
            state.status.height = image.height();
            state.image = Some(Arc::new(image));
        }
        Ok(None) => {
            drop(state);
            close(app, job);
            return;
        }
        Err(error) => {
            crate::diagnostics::record("截图", "启动长截图采集", None, &error);
            state.status.phase = Phase::Failed;
            state.status.note = Some(error);
            return;
        }
    }
    drop(state);
    if let Some(monitor) = screenshot::output_monitor(&control, Some(job.area)) {
        let work = work_area(&monitor);
        let width = ((710.0 * monitor.scale_factor()).round() as u32).min(work.width);
        let height = ((760.0 * monitor.scale_factor()).round() as u32).min(work.height);
        let _ = control.set_position(PhysicalPosition::new(
            work.x + ((work.width - width) / 2) as i32,
            work.y + ((work.height - height) / 2) as i32,
        ));
        let _ = control.set_size(PhysicalSize::new(width, height));
    }
    let _ = control.set_focus();
}

#[tauri::command]
pub fn screenshot_long_status(window: WebviewWindow) -> Result<Status, String> {
    Ok(current(&window)?.state.lock().unwrap().status.clone())
}

#[tauri::command]
pub fn screenshot_long_finish(window: WebviewWindow) -> Result<(), String> {
    let job = current(&window)?;
    let mut state = job.state.lock().unwrap();
    if state.status.phase == Phase::Scrolling {
        state.status.phase = Phase::Finishing;
        let _ = job
            .request
            .compare_exchange(CAPTURE, FINISH, Ordering::AcqRel, Ordering::Acquire);
    }
    Ok(())
}

#[tauri::command]
pub fn screenshot_long_cancel(window: WebviewWindow) -> Result<(), String> {
    let job = current(&window)?;
    let _output = job.output.try_lock().map_err(|_| "请先完成图片输出")?;
    job.request.store(CANCEL, Ordering::Release);
    window.destroy().map_err(|error| error.to_string())
}

fn preview_image(job: &Job) -> Result<Arc<RgbaImage>, String> {
    job.state
        .lock()
        .unwrap()
        .image
        .clone()
        .ok_or_else(|| "长截图仍在拼接".into())
}

#[tauri::command]
pub async fn screenshot_long_preview(window: WebviewWindow) -> Result<Response, String> {
    let job = current(&window)?;
    let image = preview_image(&job)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut bytes = Vec::new();
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|error| error.to_string())?;
        Ok(Response::new(bytes))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    Copy,
    Save,
}

#[tauri::command]
pub async fn screenshot_long_output(
    window: WebviewWindow,
    action: Output,
    top: u32,
    bottom: u32,
) -> Result<bool, String> {
    let job = current(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _output = job.output.try_lock().map_err(|_| "正在输出长截图")?;
        let source = preview_image(&job)?;
        let height = source.height().saturating_sub(top.saturating_add(bottom));
        if height == 0 {
            return Err("请保留至少一行图像".into());
        }
        let image = image::imageops::crop_imm(&*source, 0, top, source.width(), height).to_image();
        let app = window.app_handle();
        let monitor = screenshot::output_monitor(&window, Some(job.area));
        match action {
            Output::Copy => screenshot::copy_image(app, &image)?,
            Output::Save => {
                let Some(path) = screenshot::save_image(
                    &window,
                    monitor.as_ref(),
                    DynamicImage::ImageRgba8(image),
                    None,
                )?
                else {
                    return Ok(false);
                };
                screenshot::remember_output_directory(&path);
            }
        }
        let handle = app.clone();
        let done = job.clone();
        app.run_on_main_thread(move || {
            close(&handle, &done);
            if matches!(action, Output::Copy)
                && let Some(monitor) = monitor
            {
                let _ = screenshot::show_copy_toast(&handle, &monitor);
            }
        })
        .map_err(|error| error.to_string())?;
        Ok(true)
    })
    .await
    .map_err(|error| error.to_string())?
}
