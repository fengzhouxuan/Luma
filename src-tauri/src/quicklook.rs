//! External QuickLook IPC integration; no QuickLook binaries or rendering code are bundled.
use serde::Serialize;

#[derive(Serialize)]
pub struct PreviewError {
    code: &'static str,
    message: String,
}
impl From<String> for PreviewError {
    fn from(message: String) -> Self {
        Self {
            code: if message.starts_with("未找到 QuickLook") {
                "unavailable"
            } else {
                "error"
            },
            message,
        }
    }
}

fn preview_message(path: &str) -> Result<Vec<u8>, String> {
    if path.is_empty() || path.chars().count() > 32767 || path.contains(['\0', '\r', '\n', '|']) {
        return Err("文件路径包含 QuickLook 通信接口不支持的字符或长度".into());
    }
    // Toggle with the per-window `top` option invokes the preview without toggling
    // it closed. It doesn't change QuickLook's saved Topmost preference.
    Ok(format!("QuickLook.App.PipeMessages.Toggle|{path}|top\n").into_bytes())
}

#[tauri::command]
pub async fn preview_file(
    app: tauri::AppHandle,
    id: String,
    session_id: String,
    request_id: u64,
) -> Result<(), PreviewError> {
    let entry = crate::file_search::cached_entry(&id).map_err(PreviewError::from)?;
    if session_id.is_empty() || session_id.len() > 100 {
        return Err("预览会话无效".to_string().into());
    }
    let hold = crate::runtime::hold_visible();
    tauri::async_runtime::spawn_blocking(move || {
        if !std::path::Path::new(&entry.path).exists() {
            return Err("文件已移动、删除或磁盘已断开，请重新搜索".to_string());
        }
        platform::open(&app, &entry.path, &session_id, request_id, hold)
    })
    .await
    .map_err(|e| PreviewError::from(e.to_string()))?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn close_file_preview(session_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || platform::close(&session_id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn open_quicklook_download(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url("https://github.com/QL-Win/QuickLook/releases", None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        ptr::{null, null_mut},
        sync::{
            Mutex, OnceLock,
            atomic::{AtomicU64, Ordering},
        },
        time::{Duration, Instant},
    };
    use tauri::{Emitter, Manager};
    use windows_sys::Win32::{
        Foundation::*,
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        },
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, WriteFile},
        System::{
            IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
            Pipes::GetNamedPipeServerProcessId,
            Threading::{
                CreateEventW, GetCurrentProcess, OpenProcess, OpenProcessToken,
                PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, WaitForSingleObject,
            },
        },
        UI::WindowsAndMessaging::*,
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }
    fn pipe_name() -> Result<Vec<u16>, String> {
        unsafe {
            let mut token = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err("无法读取当前用户身份".into());
            }
            let token = Handle(token);
            let mut len = 0;
            GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut len);
            let mut data = vec![0usize; (len as usize).div_ceil(std::mem::size_of::<usize>())];
            if len == 0
                || GetTokenInformation(token.0, TokenUser, data.as_mut_ptr() as _, len, &mut len)
                    == 0
            {
                return Err("无法读取当前用户身份".into());
            }
            let user = &*(data.as_ptr() as *const TOKEN_USER);
            let mut sid = null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
                return Err("无法读取当前用户身份".into());
            }
            let mut count = 0;
            while *sid.add(count) != 0 {
                count += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(sid, count));
            LocalFree(sid as _);
            Ok(wide(&format!(r"\\.\pipe\QuickLook.App.Pipe.{text}")))
        }
    }
    fn connect(name: &[u16], timeout: Duration) -> Result<(Handle, u32), String> {
        let deadline = Instant::now() + timeout;
        loop {
            unsafe {
                let raw = CreateFileW(
                    name.as_ptr(),
                    GENERIC_WRITE,
                    0,
                    null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    null_mut(),
                );
                if raw != INVALID_HANDLE_VALUE {
                    let handle = Handle(raw);
                    let mut pid = 0;
                    if GetNamedPipeServerProcessId(handle.0, &mut pid) == 0
                        || !quicklook_process(pid)
                    {
                        return Err("QuickLook 通信进程身份验证失败".into());
                    }
                    return Ok((handle, pid));
                }
            }
            if Instant::now() >= deadline {
                return Err("QuickLook 尚未就绪或未运行，请启动后重试".into());
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }
    fn quicklook_process(pid: u32) -> bool {
        unsafe {
            let raw = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if raw.is_null() {
                return false;
            }
            let process = Handle(raw);
            let mut buffer = vec![0u16; 32768];
            let mut size = buffer.len() as u32;
            QueryFullProcessImageNameW(process.0, 0, buffer.as_mut_ptr(), &mut size) != 0
                && std::path::Path::new(&String::from_utf16_lossy(&buffer[..size as usize]))
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("QuickLook.exe"))
        }
    }
    fn write(handle: &Handle, bytes: &[u8]) -> Result<(), String> {
        unsafe {
            let event = Handle(CreateEventW(null(), 1, 0, null()));
            if event.0.is_null() {
                return Err("无法创建预览通信事件".into());
            }
            let mut overlapped: OVERLAPPED = std::mem::zeroed();
            overlapped.hEvent = event.0;
            let mut written = 0;
            if WriteFile(
                handle.0,
                bytes.as_ptr(),
                bytes.len() as u32,
                null_mut(),
                &mut overlapped,
            ) == 0
            {
                if GetLastError() != ERROR_IO_PENDING {
                    return Err("发送预览请求失败，请重试".into());
                }
                if WaitForSingleObject(event.0, 800) != WAIT_OBJECT_0 {
                    CancelIoEx(handle.0, &overlapped);
                    // Retain the buffer, event and OVERLAPPED until cancellation completes.
                    GetOverlappedResult(handle.0, &overlapped, &mut written, 1);
                    return Err("QuickLook 通信超时，请重试".into());
                }
            }
            if GetOverlappedResult(handle.0, &overlapped, &mut written, 0) == 0 {
                return Err("QuickLook 通信中断，请重试".into());
            }
            if written as usize != bytes.len() {
                return Err("预览请求未完整发送，请重试".into());
            }
            Ok(())
        }
    }
    fn start() -> Result<(), String> {
        for hive in [
            winreg::enums::HKEY_CURRENT_USER,
            winreg::enums::HKEY_LOCAL_MACHINE,
        ] {
            for view in [
                winreg::enums::KEY_WOW64_64KEY,
                winreg::enums::KEY_WOW64_32KEY,
            ] {
                if let Ok(key) = winreg::RegKey::predef(hive).open_subkey_with_flags(
                    r"Software\Microsoft\Windows\CurrentVersion\App Paths\QuickLook.exe",
                    winreg::enums::KEY_READ | view,
                ) {
                    if let Ok(value) = key.get_value::<String, _>("") {
                        let path = std::path::PathBuf::from(value.trim().trim_matches('"'));
                        if path.is_absolute()
                            && path.is_file()
                            && path
                                .file_name()
                                .is_some_and(|name| name.eq_ignore_ascii_case("QuickLook.exe"))
                        {
                            return crate::launch::shell_execute(
                                &path.to_string_lossy(),
                                "",
                                None,
                                false,
                            );
                        }
                    }
                }
            }
        }
        for (variable, suffix) in [
            ("LOCALAPPDATA", "Programs/QuickLook/QuickLook.exe"),
            ("ProgramFiles", "QuickLook/QuickLook.exe"),
            ("ProgramFiles(x86)", "QuickLook/QuickLook.exe"),
        ] {
            if let Some(root) = std::env::var_os(variable) {
                let path = std::path::PathBuf::from(root).join(suffix);
                if path.is_file() {
                    return crate::launch::shell_execute(&path.to_string_lossy(), "", None, false);
                }
            }
        }
        // Includes Store/MSIX and registered custom installation directories.
        if let Some(entry) = crate::catalog::scan(false).apps.into_iter().find(|entry| {
            entry.name.eq_ignore_ascii_case("QuickLook")
                && (crate::packaged::app_id(&entry.path).is_some()
                    || std::path::Path::new(&entry.path)
                        .file_name()
                        .is_some_and(|name| name.eq_ignore_ascii_case("QuickLook.exe")))
        }) {
            return crate::launch::launch_entry(&entry, false);
        }
        Err("未找到 QuickLook，请先安装，或手动启动便携版后重试".into())
    }
    struct Session {
        owner: String,
        request_id: u64,
        generation: u64,
        hwnd: usize,
        pid: u32,
        name: String,
        _hold: crate::runtime::VisibilityHold,
    }
    static STATE: OnceLock<Mutex<Option<Session>>> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(1);
    fn state() -> &'static Mutex<Option<Session>> {
        STATE.get_or_init(Default::default)
    }
    fn window_matches(hwnd: HWND, pid: u32, name: &str) -> bool {
        unsafe {
            let mut actual = 0;
            GetWindowThreadProcessId(hwnd, &mut actual);
            let mut class = [0u16; 256];
            let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
            let mut title = [0u16; 32768];
            let title_len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
            actual == pid
                && IsWindowVisible(hwnd) != 0
                && len > 0
                && String::from_utf16_lossy(&class[..len as usize]).starts_with("HwndWrapper[")
                && title_len > 0
                && String::from_utf16_lossy(&title[..title_len as usize]).contains(name)
        }
    }
    struct Find<'a> {
        pid: u32,
        name: &'a str,
        hwnd: usize,
    }
    unsafe extern "system" fn enumerate(hwnd: HWND, data: LPARAM) -> BOOL {
        let search = unsafe { &mut *(data as *mut Find<'_>) };
        if window_matches(hwnd, search.pid, search.name) {
            search.hwnd = hwnd as usize;
            return 0;
        }
        1
    }
    fn find(pid: u32, name: &str) -> usize {
        let mut search = Find { pid, name, hwnd: 0 };
        unsafe {
            EnumWindows(Some(enumerate), &mut search as *mut _ as LPARAM);
        }
        search.hwnd
    }
    fn arrange(hwnd: HWND, main: HWND) {
        unsafe {
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            let mut rect: RECT = std::mem::zeroed();
            if GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut info) != 0
                && GetWindowRect(hwnd, &mut rect) != 0
            {
                let area = info.rcWork;
                let width = (rect.right - rect.left)
                    .min(area.right - area.left - 24)
                    .max(100);
                let height = (rect.bottom - rect.top)
                    .min(area.bottom - area.top - 24)
                    .max(100);
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    area.left + (area.right - area.left - width) / 2,
                    area.top + (area.bottom - area.top - height) / 2,
                    width,
                    height,
                    SWP_NOACTIVATE,
                );
            }
        }
    }
    fn send_close(session: &Session) -> Result<(), String> {
        if !window_matches(session.hwnd as HWND, session.pid, &session.name) {
            return Ok(());
        }
        let (handle, pid) = connect(&pipe_name()?, Duration::from_millis(200))?;
        if pid != session.pid {
            return Ok(());
        }
        write(&handle, b"QuickLook.App.PipeMessages.Close||\n")
    }
    pub fn close(owner: &str) -> Result<(), String> {
        let mut guard = state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.as_ref().is_some_and(|session| session.owner == owner) {
            let result = send_close(guard.as_ref().unwrap());
            // Retain the visibility hold on a real communication error; retry is possible.
            if result.is_ok() {
                guard.take();
            }
            result
        } else {
            Ok(())
        }
    }
    pub fn open(
        app: &tauri::AppHandle,
        path: &str,
        owner: &str,
        request_id: u64,
        hold: crate::runtime::VisibilityHold,
    ) -> Result<(), String> {
        let bytes = preview_message(path)?;
        let main = app.get_webview_window("main").ok_or("启动器窗口已关闭")?;
        if !main.is_visible().unwrap_or(false) {
            return Err("启动器已收起，预览已取消".into());
        }
        let mut guard = state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("无效的文件路径")?
            .to_owned();
        let pipe = pipe_name()?;
        let (handle, pid) = match connect(&pipe, Duration::from_millis(120)) {
            Ok(value) => value,
            Err(_) => {
                start()?;
                connect(&pipe, Duration::from_secs(5))?
            }
        };
        if !main.is_visible().unwrap_or(false) {
            return Err("启动器已收起，预览已取消".into());
        }
        write(&handle, &bytes)?;
        drop(handle);
        let deadline = Instant::now() + Duration::from_secs(4);
        let hwnd = loop {
            let hwnd = find(pid, &name);
            let visible = main.is_visible().unwrap_or(false);
            if hwnd != 0 && visible {
                break hwnd;
            }
            if !visible || Instant::now() >= deadline {
                // Cancel a delayed QL dispatcher operation too, so an unsuccessful
                // attempt cannot pop up after Luma has reported it as closed.
                if let Ok((handle, current_pid)) = connect(&pipe, Duration::from_millis(200)) {
                    if current_pid == pid {
                        let _ = write(&handle, b"QuickLook.App.PipeMessages.Close||\n");
                    }
                }
                return Err("QuickLook 未显示预览窗口，请确认文件格式受支持后重试".into());
            }
            std::thread::sleep(Duration::from_millis(60));
        };
        let main_hwnd = main.hwnd().map_err(|e| e.to_string())?.0 as HWND;
        let generation = NEXT.fetch_add(1, Ordering::Relaxed);
        *guard = Some(Session {
            owner: owner.into(),
            request_id,
            generation,
            hwnd,
            pid,
            name,
            _hold: hold,
        });
        drop(guard);
        // Keep result-list keyboard navigation in Luma, even if QL's optional
        // FocusWindowOnOpen preference activated the viewer.
        let foreground = unsafe { GetForegroundWindow() };
        let mut foreground_pid = 0;
        unsafe {
            GetWindowThreadProcessId(foreground, &mut foreground_pid);
        }
        if foreground == main_hwnd || foreground_pid == pid {
            let _ = main.set_focus();
        }
        arrange(hwnd as HWND, main_hwnd);
        let app = app.clone();
        std::thread::spawn(move || monitor(app, generation));
        Ok(())
    }
    fn monitor(app: tauri::AppHandle, generation: u64) {
        loop {
            std::thread::sleep(Duration::from_millis(180));
            let Ok(mut guard) = state().try_lock() else {
                continue;
            };
            let Some(session) = guard
                .as_ref()
                .filter(|session| session.generation == generation)
            else {
                return;
            };
            let main = app.get_webview_window("main");
            let foreground = unsafe { GetForegroundWindow() };
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(foreground, &mut pid);
            }
            let visible = window_matches(session.hwnd as HWND, session.pid, &session.name);
            let main_visible = main
                .as_ref()
                .is_some_and(|main| main.is_visible().unwrap_or(false));
            let outside = pid != 0 && pid != std::process::id() && pid != session.pid;
            if !visible || !main_visible || outside {
                if visible {
                    let _ = send_close(session);
                }
                #[derive(Clone, Serialize)]
                struct Closed {
                    session_id: String,
                    request_id: u64,
                }
                let closed = Closed {
                    session_id: session.owner.clone(),
                    request_id: session.request_id,
                };
                guard.take();
                drop(guard);
                let _ = app.emit("quicklook-preview-closed", closed);
                if outside && main_visible {
                    crate::hide_window(app.clone());
                }
                return;
            }
        }
    }
    #[cfg(test)]
    mod transport_tests {
        use super::*;
        #[test]
        fn a_server_that_stops_reading_times_out_and_cancels_the_write() {
            use windows_sys::Win32::{
                Storage::FileSystem::PIPE_ACCESS_INBOUND, System::Pipes::CreateNamedPipeW,
            };
            unsafe {
                let name = wide(&format!(
                    r"\\.\pipe\Luma-QuickLook-Test-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                let server = Handle(CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_INBOUND,
                    0,
                    1,
                    0,
                    4,
                    0,
                    null(),
                ));
                assert_ne!(server.0, INVALID_HANDLE_VALUE);
                let client = Handle(CreateFileW(
                    name.as_ptr(),
                    GENERIC_WRITE,
                    0,
                    null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    null_mut(),
                ));
                assert_ne!(client.0, INVALID_HANDLE_VALUE);
                let started = Instant::now();
                assert!(
                    write(&client, &vec![b'x'; 1024 * 1024])
                        .unwrap_err()
                        .contains("超时")
                );
                assert!(started.elapsed() < Duration::from_secs(3));
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn open(
        _: &tauri::AppHandle,
        _: &str,
        _: &str,
        _: u64,
        _: crate::runtime::VisibilityHold,
    ) -> Result<(), String> {
        Err("QuickLook 预览仅支持 Windows".into())
    }
    pub fn close(_: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_preserves_unicode_spaces_and_shell_metacharacters_but_rejects_frames() {
        assert_eq!(
            preview_message(r"D:\文档 空格\报价 & (新).txt").unwrap(),
            "QuickLook.App.PipeMessages.Toggle|D:\\文档 空格\\报价 & (新).txt|top\n".as_bytes()
        );
        for path in [
            "",
            "x\0y",
            "x\ny",
            "x\ry",
            "x|QuickLook.App.PipeMessages.Quit",
        ] {
            assert!(preview_message(path).is_err());
        }
        assert!(preview_message(&"x".repeat(32768)).is_err());
    }
}
