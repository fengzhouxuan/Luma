//! On-demand window discovery. Window titles and handles never enter config or logs.
use crate::model::{AppEntry, EntryKind};
use serde::Serialize;

#[derive(Serialize)]
pub struct RunningWindow {
    id: String,
    title: String,
    minimized: bool,
}

#[derive(Serialize)]
pub struct RunningWindows {
    windows: Vec<RunningWindow>,
    can_new_window: bool,
}

// Only advertise a new window where the application documents that operation.
// Parameterized entries keep their original launch semantics instead of guessing
// how --reuse-window, task commands, profiles or URI handlers should be combined.
pub(crate) fn can_new_window(app: &AppEntry) -> bool {
    app.kind == EntryKind::App
        && app.arguments.trim().is_empty()
        && std::path::Path::new(&app.path)
            .file_name()
            .is_some_and(|name| {
                name.eq_ignore_ascii_case("Code.exe")
                    || name.eq_ignore_ascii_case("Code - Insiders.exe")
            })
}

#[tauri::command]
pub async fn get_running_windows(app_id: String) -> Result<RunningWindows, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let app = crate::catalog::find(&app_id).ok_or("应用记录已失效，请刷新后重试")?;
        Ok(RunningWindows {
            windows: platform::list(&app),
            can_new_window: can_new_window(&app),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn activate_app_window(app_id: String, window_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let app = crate::catalog::find(&app_id).ok_or("应用记录已失效，请刷新后重试")?;
        platform::activate(&app, &window_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        collections::HashMap,
        sync::{
            Mutex, OnceLock,
            atomic::{AtomicU64, Ordering},
        },
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{BOOL, CloseHandle, FILETIME, HWND, LPARAM},
        Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
        Storage::Packaging::Appx::GetApplicationUserModelId,
        System::Threading::{
            GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::WindowsAndMessaging::*,
    };

    #[derive(Clone, PartialEq, Eq)]
    enum Identity {
        Exe(String),
        Packaged(String),
    }
    #[derive(Clone)]
    struct Proof {
        app_id: String,
        hwnd: usize,
        pid: u32,
        born: u64,
        expires: Instant,
    }
    static PROOFS: OnceLock<Mutex<HashMap<String, Proof>>> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(1);
    fn proofs() -> &'static Mutex<HashMap<String, Proof>> {
        PROOFS.get_or_init(Default::default)
    }
    fn path_key(path: &str) -> String {
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.into())
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('/', "\\")
            .to_lowercase()
    }
    fn target(app: &AppEntry) -> Option<Identity> {
        if app.kind != EntryKind::App {
            return None;
        }
        if let Some(id) = crate::packaged::app_id(&app.path) {
            return Some(Identity::Packaged(id.to_lowercase()));
        }
        let path = std::path::Path::new(&app.path);
        (path.is_absolute()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe")))
        .then(|| Identity::Exe(path_key(&app.path)))
    }
    // Every successful OpenProcess is paired with CloseHandle, including errors.
    fn process(pid: u32) -> Option<(String, Option<String>, u64)> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let result = (|| {
                let mut path = vec![0u16; 32768];
                let mut len = path.len() as u32;
                if QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut len) == 0 {
                    return None;
                }
                let mut born: FILETIME = std::mem::zeroed();
                let mut exit: FILETIME = std::mem::zeroed();
                let mut kernel: FILETIME = std::mem::zeroed();
                let mut user: FILETIME = std::mem::zeroed();
                if GetProcessTimes(handle, &mut born, &mut exit, &mut kernel, &mut user) == 0 {
                    return None;
                }
                let mut id = vec![0u16; 512];
                let mut size = id.len() as u32;
                let aumid = (GetApplicationUserModelId(handle, &mut size, id.as_mut_ptr()) == 0)
                    .then(|| {
                        String::from_utf16_lossy(
                            &id[..(size.saturating_sub(1) as usize).min(id.len())],
                        )
                        .to_lowercase()
                    });
                Some((
                    path_key(&String::from_utf16_lossy(&path[..len as usize])),
                    aumid,
                    ((born.dwHighDateTime as u64) << 32) | born.dwLowDateTime as u64,
                ))
            })();
            CloseHandle(handle);
            result
        }
    }
    fn usable(hwnd: HWND) -> bool {
        unsafe {
            let mut cloaked: u32 = 0;
            DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, &mut cloaked as *mut _ as _, 4);
            IsWindow(hwnd) != 0
                && IsWindowVisible(hwnd) != 0
                && cloaked == 0
                && GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW == 0
                && GetWindow(hwnd, GW_OWNER).is_null()
        }
    }
    fn identity_matches(target: &Identity, path: &str, aumid: Option<&str>) -> bool {
        match target {
            Identity::Exe(exe) => exe == path,
            Identity::Packaged(id) => aumid == Some(id.as_str()),
        }
    }
    struct ChildSearch<'a> {
        target: &'a Identity,
        matched: bool,
    }
    unsafe extern "system" fn child(hwnd: HWND, data: LPARAM) -> BOOL {
        let search = unsafe { &mut *(data as *mut ChildSearch<'_>) };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if let Some((path, id, _)) = process(pid) {
            search.matched |= identity_matches(search.target, &path, id.as_deref());
        }
        (!search.matched) as BOOL
    }
    fn matches(hwnd: HWND, target: &Identity) -> Option<(u32, u64)> {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if pid == std::process::id() {
            return None;
        }
        let (path, id, born) = process(pid)?;
        if identity_matches(target, &path, id.as_deref()) {
            return Some((pid, born));
        }
        // Classic UWP hosts expose the application identity on their child process.
        let host = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        if matches!(target, Identity::Packaged(_))
            && path == path_key(&format!(r"{host}\System32\ApplicationFrameHost.exe"))
        {
            let mut search = ChildSearch {
                target,
                matched: false,
            };
            unsafe {
                EnumChildWindows(hwnd, Some(child), &mut search as *mut _ as LPARAM);
            }
            if search.matched {
                return Some((pid, born));
            }
        }
        None
    }
    struct Search {
        target: Identity,
        app_id: String,
        windows: Vec<RunningWindow>,
        proofs: Vec<(String, Proof)>,
    }
    unsafe extern "system" fn enumerate(hwnd: HWND, data: LPARAM) -> BOOL {
        let search = unsafe { &mut *(data as *mut Search) };
        if search.windows.len() >= 64 {
            return 0;
        }
        if !usable(hwnd) {
            return 1;
        }
        if let Some((pid, born)) = matches(hwnd, &search.target) {
            let mut title = [0u16; 1024];
            let len = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
            if len > 0 {
                let id = NEXT.fetch_add(1, Ordering::Relaxed).to_string();
                search.proofs.push((
                    id.clone(),
                    Proof {
                        app_id: search.app_id.clone(),
                        hwnd: hwnd as usize,
                        pid,
                        born,
                        expires: Instant::now() + Duration::from_secs(300),
                    },
                ));
                search.windows.push(RunningWindow {
                    id,
                    title: String::from_utf16_lossy(&title[..len as usize]),
                    minimized: unsafe { IsIconic(hwnd) != 0 },
                });
            }
        }
        1
    }
    pub fn list(app: &AppEntry) -> Vec<RunningWindow> {
        let Some(target) = target(app) else {
            return Vec::new();
        };
        let mut search = Search {
            target,
            app_id: app.id.clone(),
            windows: Vec::new(),
            proofs: Vec::new(),
        };
        unsafe {
            EnumWindows(Some(enumerate), &mut search as *mut _ as LPARAM);
        }
        let mut cache = proofs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.retain(|_, proof| proof.app_id != app.id && proof.expires > Instant::now());
        if cache.len() + search.proofs.len() > 256 {
            cache.clear();
        }
        cache.extend(search.proofs);
        search.windows
    }
    pub fn activate(app: &AppEntry, id: &str) -> Result<(), String> {
        let stale = "窗口已关闭或发生变化，请刷新窗口列表";
        let proof = proofs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .filter(|proof| proof.app_id == app.id && proof.expires > Instant::now())
            .cloned()
            .ok_or(stale)?;
        let hwnd = proof.hwnd as HWND;
        let target = target(app).ok_or(stale)?;
        if !usable(hwnd) || matches(hwnd, &target) != Some((proof.pid, proof.born)) {
            return Err(stale.into());
        }
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindowAsync(hwnd, SW_RESTORE);
            }
            // Prefer an owned modal dialog so a disabled main window isn't focused.
            let popup = GetLastActivePopup(hwnd);
            let focus = if !popup.is_null()
                && IsWindowVisible(popup) != 0
                && GetAncestor(popup, GA_ROOTOWNER) == hwnd
            {
                popup
            } else {
                hwnd
            };
            SetForegroundWindow(focus);
            for _ in 0..10 {
                if GetForegroundWindow() == focus {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        Err("Windows 未允许切换到该窗口，请重试或从任务栏打开".into())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        #[ignore = "读取本机测试窗口；通过 LUMA_WINDOW_FIXTURE 指定专用测试程序"]
        fn host_window_discovery_and_stale_selection() {
            let path = std::env::var("LUMA_WINDOW_FIXTURE").expect("fixture path required");
            let app = AppEntry::new("Luma fixture".into(), path, "test", "".into());
            let first = list(&app);
            assert!(
                first.len() >= 2,
                "expected two visible fixture windows, got {}",
                first.len()
            );
            assert!(
                first
                    .iter()
                    .all(|window| window.title.starts_with("Luma Window QA"))
            );
            assert!(first.iter().any(|window| window.minimized));
            let mut other = app.clone();
            other.id = "different app".into();
            assert!(
                activate(&other, &first[0].id)
                    .unwrap_err()
                    .contains("窗口已关闭")
            );
            let second = list(&app);
            assert_eq!(first.len(), second.len());
            assert!(
                activate(&app, &first[0].id)
                    .unwrap_err()
                    .contains("窗口已关闭")
            );
            assert!(activate(&app, "untrusted-window-id").is_err());
        }
        #[test]
        fn does_not_match_same_named_exes_or_other_package_apps() {
            let target = Identity::Exe(path_key(r"C:\Apps\Editor.exe"));
            assert!(identity_matches(
                &target,
                &path_key(r"c:\apps\EDITOR.exe"),
                None
            ));
            assert!(!identity_matches(
                &target,
                &path_key(r"D:\Apps\Editor.exe"),
                None
            ));
            assert!(!identity_matches(
                &Identity::Packaged("family!app".into()),
                "",
                Some("family!other")
            ));
        }
        #[test]
        fn excludes_non_application_entries_and_protocols() {
            let mut app = AppEntry::new(
                "fixture".into(),
                "https://example.com".into(),
                "test",
                "".into(),
            );
            assert!(target(&app).is_none());
            app.path = r"C:\Apps\fixture.exe".into();
            assert!(target(&app).is_some());
            app.kind = EntryKind::Script;
            assert!(target(&app).is_none());
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn list(_: &AppEntry) -> Vec<RunningWindow> {
        Vec::new()
    }
    pub fn activate(_: &AppEntry, _: &str) -> Result<(), String> {
        Err("当前平台不支持窗口切换".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_window_requires_supported_plain_entry() {
        let mut app = AppEntry::new(
            "editor".into(),
            r"C:\Code\Code.exe".into(),
            "test",
            "".into(),
        );
        assert!(can_new_window(&app));
        app.arguments = "--reuse-window \"C:\\work\"".into();
        assert!(!can_new_window(&app));
        app.arguments.clear();
        app.path = r"C:\Other\Other.exe".into();
        assert!(!can_new_window(&app));
    }
}
