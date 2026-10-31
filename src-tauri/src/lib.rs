use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Serialize, Clone)]
struct AppEntry {
    name: String,
    path: String,
    /// 图标的 base64 data URL（如 "data:image/png;base64,..."），提取失败则为 None
    icon: Option<String>,
}

/// 扫描应用：开始菜单 .lnk + 注册表 App Paths
#[tauri::command]
fn scan_apps() -> Vec<AppEntry> {
    let mut apps: Vec<AppEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // 1. 扫描开始菜单（用户 + 公共）
    let mut start_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(programs) = std::env::var("APPDATA") {
        start_dirs.push(PathBuf::from(programs).join("Microsoft/Windows/Start Menu/Programs"));
    }
    if let Ok(program_data) = std::env::var("ProgramData") {
        start_dirs.push(PathBuf::from(program_data).join("Microsoft/Windows/Start Menu/Programs"));
    }

    for dir in start_dirs {
        let pattern = dir.join("**/*.lnk").to_string_lossy().to_string();
        if let Ok(paths) = glob::glob(&pattern) {
            for entry in paths.flatten() {
                if let Some(app) = parse_lnk(&entry) {
                    if seen.insert(app.name.to_lowercase()) {
                        apps.push(app);
                    }
                }
            }
        }
    }

    // 2. 扫描注册表 App Paths（补充未在开始菜单的程序）
    scan_app_paths(&mut apps, &mut seen);

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

/// 用 parselnk 解析 .lnk 快捷方式
fn parse_lnk(lnk_path: &PathBuf) -> Option<AppEntry> {
    let name = lnk_path.file_stem()?.to_string_lossy().to_string();

    let bytes = std::fs::read(lnk_path).ok()?;
    let shortcut = parselnk::Lnk::try_from(bytes.as_slice()).ok()?;

    // 解析真正的目标路径：优先 link_info 里的完整路径，再回退 relative_path
    let target_str = resolve_lnk_target(&shortcut, lnk_path)?;

    // 注意：图标不在扫描阶段提取（避免阻塞），由前端通过 get_icon 命令异步加载
    Some(AppEntry {
        name,
        path: target_str,
        icon: None,
    })
}

/// 图标提取缓存：path + 修改时间 → base64 data URL
/// 避免每次启动都重新读取 exe 的 PE 资源并编码 base64
#[cfg(windows)]
static ICON_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, Option<String>>>,
> = std::sync::OnceLock::new();

/// 获取应用的图标（base64 data URL），带内存缓存。
/// 供前端异步调用，避免扫描阶段阻塞 UI。
#[tauri::command]
fn get_icon(path: String) -> Option<String> {
    #[cfg(windows)]
    {
        use std::collections::HashMap;
        let cache = ICON_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));

        // 用路径 + 文件修改时间做缓存 key，文件更新后自动失效
        let mtime = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .map(|t| format!("{:?}", t))
            .unwrap_or_default();
        let cache_key = format!("{}|{}", path, mtime);

        // 先查缓存
        if let Some(icon) = cache.lock().unwrap().get(&cache_key) {
            return icon.clone();
        }

        // 提取图标
        let icon = extract_icon_base64(&path);

        // 写入缓存
        cache.lock().unwrap().insert(cache_key, icon.clone());
        icon
    }

    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

/// 从 .lnk 中解析出完整的目标路径
fn resolve_lnk_target(shortcut: &parselnk::Lnk, lnk_path: &PathBuf) -> Option<String> {
    // 1. LinkInfo 里的完整路径（最可靠）
    if let Some(p) = shortcut
        .link_info
        .local_base_path_unicode
        .as_ref()
        .or(shortcut.link_info.local_base_path.as_ref())
    {
        let p = p.trim().to_string();
        if !p.is_empty() && std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }

    // 2. relative_path + working_dir 拼接
    if let Some(rel) = shortcut.relative_path() {
        let rel = rel.to_string_lossy().to_string();
        let full = if let Some(wd) = shortcut.working_dir() {
            wd.join(&rel)
        } else {
            std::path::PathBuf::from(&rel)
        };
        let full = full.to_string_lossy().to_string();
        if !full.is_empty() && std::path::Path::new(&full).exists() {
            return Some(full);
        }
        // relative_path 本身可能就是完整路径
        if std::path::Path::new(&rel).exists() {
            return Some(rel);
        }
    }

    // 3. 兜底：相对 .lnk 所在目录解析
    if let Some(rel) = shortcut.relative_path() {
        if let Some(parent) = lnk_path.parent() {
            let full = parent.join(rel);
            let full = full.to_string_lossy().to_string();
            if std::path::Path::new(&full).exists() {
                return Some(full);
            }
        }
    }

    None
}

/// 从文件路径提取图标，返回 base64 data URL
#[cfg(windows)]
fn extract_icon_base64(path: &str) -> Option<String> {
    use std::path::Path;
    if !Path::new(path).exists() {
        return None;
    }
    windows_icons::get_icon_base64_by_path_with_size(path, windows_icons::IconSize::Large)
        .ok()
        .map(|b64| format!("data:image/png;base64,{}", b64))
}

#[cfg(not(windows))]
fn extract_icon_base64(_path: &str) -> Option<String> {
    None
}

/// 扫描注册表 App Paths
#[cfg(windows)]
fn scan_app_paths(apps: &mut Vec<AppEntry>, seen: &mut HashSet<String>) {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let app_paths = match RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths")
    {
        Ok(k) => k,
        Err(_) => return,
    };

    for key in app_paths.enum_keys().flatten() {
        if key.to_lowercase().ends_with(".exe") {
            if let Ok(subkey) = app_paths.open_subkey(&key) {
                if let Ok(path) = subkey.get_value::<String, _>("") {
                    // 优先用 exe 版本资源里的 FileDescription 作为友好显示名
                    // （如 "Google Chrome"、"Microsoft Edge"），失败则回退文件名
                    let name = friendly_name(&path)
                        .unwrap_or_else(|| key.trim_end_matches(".exe").to_string());

                    if seen.insert(name.to_lowercase()) {
                        apps.push(AppEntry { name, path, icon: None });
                    }
                }
            }
        }
    }
}

/// 从 exe 的版本资源读取 FileDescription（友好显示名）
#[cfg(windows)]
fn friendly_name(path: &str) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };

    let wide: Vec<u16> = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let size = GetFileVersionInfoSizeW(wide.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return None;
        }

        let mut buffer = vec![0u8; size as usize];
        if GetFileVersionInfoW(wide.as_ptr(), 0, size, buffer.as_mut_ptr() as _) == 0 {
            return None;
        }

        // 构造查询字符串：\StringFileInfo\<lang>\FileDescription
        // 常见语言代码：040904B0（英文 US）、080404B0（中文简体）
        for lang in ["040904B0", "080404B0"] {
            let sub_block: Vec<u16> = format!("\\StringFileInfo\\{}\\FileDescription\0", lang)
                .encode_utf16()
                .collect();

            let mut value_ptr: *mut u16 = std::ptr::null_mut();
            let mut value_len: u32 = 0;

            if VerQueryValueW(
                buffer.as_ptr() as _,
                sub_block.as_ptr(),
                &mut value_ptr as *mut _ as *mut _,
                &mut value_len,
            ) != 0
                && !value_ptr.is_null()
                && value_len > 0
            {
                let slice = std::slice::from_raw_parts(value_ptr, value_len as usize);
                let desc = String::from_utf16_lossy(slice).trim().to_string();
                if !desc.is_empty() {
                    return Some(desc);
                }
            }
        }

        None
    }
}

#[cfg(not(windows))]
fn scan_app_paths(_apps: &mut Vec<AppEntry>, _seen: &mut HashSet<String>) {}

#[cfg(not(windows))]
fn friendly_name(_path: &str) -> Option<String> {
    None
}

/// 启动应用（可选管理员权限）
#[tauri::command]
fn launch_app(path: String, admin: bool) -> Result<(), String> {
    launch_with_shell_execute(&path, admin)
}

/// 前端请求隐藏窗口（Esc 主路径）。同步复位权威显示标志，避免与后端状态错乱。
#[tauri::command]
fn hide_window(app: tauri::AppHandle) {
    use tauri::Manager;
    double_alt::WINDOW_VISIBLE.store(false, std::sync::atomic::Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[cfg(windows)]
fn launch_with_shell_execute(path: &str, admin: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let operation: Vec<u16> = if admin {
        "runas\0".encode_utf16().collect()
    } else {
        "open\0".encode_utf16().collect()
    };

    let file: Vec<u16> = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL as i32,
        )
    };

    if (result as isize) > 32 {
        Ok(())
    } else {
        Err(format!("启动失败，错误码: {}", result as isize))
    }
}

#[cfg(not(windows))]
fn launch_with_shell_execute(_path: &str, _admin: bool) -> Result<(), String> {
    Err("当前平台不支持".to_string())
}

/// 强制窗口获得前台焦点（PowerToys Run 同款方案）
///
/// 参考 microsoft/PowerToys 的 MainWindow.BringProcessToForeground()：
/// 发送一个"空鼠标事件"（INPUT_MOUSE 类型，鼠标数据全零），
/// 使当前线程成为"最后一个发送输入"的线程，从而通过 Windows 前台权限检查，
/// 随后 SetForegroundWindow / Activate 才能生效。
/// 此方案比"模拟 Alt 键"更可靠：空鼠标事件不触发任何实际动作，
/// 不会干扰键盘钩子，也不会激活前台窗口的菜单栏。
/// 参考 issue: https://github.com/microsoft/PowerToys/issues/4270
#[cfg(windows)]
fn force_foreground(hwnd: isize) {
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, SetActiveWindow, SetFocus, INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow, SetWindowPos,
        HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE,
    };

    unsafe {
        let hwnd = hwnd as _;

        // 1. 置顶，确保窗口在最上层
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);

        // 2. 若尚未前台，走完整抢焦点流程
        if GetForegroundWindow() != hwnd {
            let fg = GetForegroundWindow();
            let cur_thread = GetCurrentThreadId();

            // 2a. 发送空鼠标事件（PowerToys 同款技巧），让本线程成为"最后输入线程"
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: 0,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);

            // 2b. AttachThreadInput 绑定前台窗口线程的输入队列（突破 UIPI/前台锁）
            if !fg.is_null() {
                let fg_thread = GetWindowThreadProcessId(fg, std::ptr::null_mut());
                if fg_thread != 0 && fg_thread != cur_thread {
                    AttachThreadInput(cur_thread, fg_thread, 1);
                    SetForegroundWindow(hwnd);
                    AttachThreadInput(cur_thread, fg_thread, 0);
                } else {
                    SetForegroundWindow(hwnd);
                }
            } else {
                SetForegroundWindow(hwnd);
            }

            // 2c. 兜底：确保窗口为活动窗口并拥有键盘焦点
            SetActiveWindow(hwnd);
            SetFocus(hwnd);
        }
    }
}

#[cfg(not(windows))]
fn force_foreground(_hwnd: isize) {}

/// 关闭 Windows 11 的强制圆角 + 系统阴影，避免透明窗口四角残留线框/直角残影
///
/// - Windows 11 会给所有顶层窗口强制加圆角（DWMWCP_ROUND），
///   对透明窗口会在圆角外侧留下深色线框残影。
/// - decorations:false 只是去掉标题栏，系统 DropShadow 阴影仍会存在，
///   在圆角外四角形成深色角块。
#[cfg(windows)]
fn disable_window_corner_and_shadow(hwnd: isize) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOREDIRECTIONBITMAP,
    };

    unsafe {
        let hwnd = hwnd as _;

        // 1. 关闭 Windows 11 强制圆角（值 33 = DWMWCP_DONOTROUND）
        let corner_pref: i32 = DWMWCP_DONOTROUND as i32;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &corner_pref as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );

        // 2. 关闭 WS_EX_NOREDIRECTIONBITMAP，让透明区域真正透明（去除阴影/线框）
        //    有些 WebView2 透明窗口需要这一步才能彻底消除四角残影
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if ex_style != 0 {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style & !(WS_EX_NOREDIRECTIONBITMAP as isize));
        }
    }
}

#[cfg(not(windows))]
fn disable_window_corner_and_shadow(_hwnd: isize) {}

// ==================== 双击 Alt 呼出（低级键盘钩子）====================

#[cfg(windows)]
mod double_alt {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_LMENU, VK_RMENU};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG,
        WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
    };

    /// 全局标志：检测到双击 Alt 时置 true，主线程读取后复位
    pub static TOGGLE: AtomicBool = AtomicBool::new(false);

    /// 全局标志：检测到 Esc 按下时置 true，主线程读取后复位（用于隐藏窗口）
    pub static ESC_PRESSED: AtomicBool = AtomicBool::new(false);

    /// 标记窗口是否处于"由我们主动显示"的状态（权威来源，避免 is_visible 状态不一致）
    pub static WINDOW_VISIBLE: AtomicBool = AtomicBool::new(false);

    /// 上次 Alt 按下时间（用于判断是否"双击"）
    static LAST_ALT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

    unsafe extern "system" fn keyboard_proc(
        n_code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if n_code >= 0 {
            let kb = unsafe { &*(l_param as *const KBDLLHOOKSTRUCT) };
            let is_down =
                w_param == WM_KEYDOWN as usize || w_param == WM_SYSKEYDOWN as usize;
            let is_alt = kb.vkCode == VK_LMENU as u32 || kb.vkCode == VK_RMENU as u32;

            if is_down && is_alt {
                let last = LAST_ALT.get_or_init(|| Mutex::new(None));
                let mut guard = last.lock().unwrap();
                let now = Instant::now();
                let is_double = match *guard {
                    Some(prev) => now.duration_since(prev) < Duration::from_millis(400),
                    None => false,
                };
                *guard = Some(now);

                if is_double {
                    TOGGLE.store(true, Ordering::SeqCst);
                }
            }

            // 检测 Esc 按下（兜底方案）：
            // 注意：焦点在本窗口（WebView2/Chromium）时，Windows 会静默停止
            // 向 WH_KEYBOARD_LL 钩子投递事件，因此 Esc 主要由前端 keydown 处理。
            // 这里的钩子只作为"焦点不在本窗口"时的兜底。
            if is_down && kb.vkCode == VK_ESCAPE as u32 {
                ESC_PRESSED.store(true, Ordering::SeqCst);
            }
        }
        unsafe { CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param) }
    }

    /// 启动键盘钩子线程（阻塞在消息循环里）
    pub fn start() {
        std::thread::spawn(|| unsafe {
            let h_module = GetModuleHandleW(std::ptr::null());
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), h_module, 0);
            if hook.is_null() {
                return;
            }

            let mut msg: MSG = std::mem::zeroed();
            loop {
                let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                if ret <= 0 {
                    break;
                }
            }

            UnhookWindowsHookEx(hook);
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 已有实例在运行：唤醒它的窗口，新实例随后自动退出
            use std::sync::atomic::Ordering;
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                double_alt::WINDOW_VISIBLE.store(true, Ordering::SeqCst);
                let _ = window.show();
                if let Ok(hwnd) = window.hwnd() {
                    force_foreground(hwnd.0 as isize);
                }
            }
        }))
        .invoke_handler(tauri::generate_handler![scan_apps, launch_app, get_icon, hide_window])
        .setup(|app| {
            #[cfg(windows)]
            {
                use std::sync::atomic::Ordering;
                use tauri::Manager;

                // 在 setup 阶段获取窗口（此时窗口已就绪）
                let window = app.get_webview_window("main");

                if let Some(window) = window {
                    // 关闭 Windows 11 强制圆角 + 系统阴影，避免透明窗口四角残留线框
                    #[cfg(windows)]
                    if let Ok(hwnd) = window.hwnd() {
                        disable_window_corner_and_shadow(hwnd.0 as isize);
                    }

                    // 失焦自动隐藏（保持后台常驻，不退出）
                    // 采用"延迟二次确认"：收到失焦后延迟 200ms 再检查是否仍失焦，
                    // 若期间重新获得焦点则取消隐藏。既规避 show→抢焦点过渡期的抖动，
                    // 也能正确处理"窗口显示后立即点外面"的场景。
                    {
                        let win = window.clone();
                        window.on_window_event(move |event| {
                            if let tauri::WindowEvent::Focused(false) = event {
                                let win = win.clone();
                                std::thread::spawn(move || {
                                    std::thread::sleep(std::time::Duration::from_millis(200));
                                    // 再次确认是否仍失焦，且窗口当前"由我们主动显示"（权威标志）
                                    if let Ok(false) = win.is_focused() {
                                        if double_alt::WINDOW_VISIBLE.load(Ordering::SeqCst) {
                                            double_alt::WINDOW_VISIBLE.store(false, Ordering::SeqCst);
                                            let _ = win.hide();
                                        }
                                    }
                                });
                            }
                        });
                    }

                    // 启动双击 Alt 监听
                    double_alt::start();

                    // 轮询双击 Alt 标志，切换到主线程执行显示/隐藏
                    // 关键：show + 抢焦点必须在主线程（UI 线程）执行，
                    // 否则 SetForegroundWindow 受前台锁限制导致焦点抢不到
                    let win = window.clone();
                    let handle = app.handle().clone();
                    std::thread::spawn(move || loop {
                        // 处理 Esc（兜底）：窗口可见时隐藏。
                        // 主要路径在前端 keydown 处理，这里兜底"焦点不在本窗口"的场景。
                        if double_alt::ESC_PRESSED.swap(false, Ordering::SeqCst) {
                            if double_alt::WINDOW_VISIBLE.load(Ordering::SeqCst) {
                                double_alt::WINDOW_VISIBLE.store(false, Ordering::SeqCst);
                                let win = win.clone();
                                let handle = handle.clone();
                                let _ = handle.run_on_main_thread(move || {
                                    let _ = win.hide();
                                });
                            }
                        }

                        // 处理双击 Alt：切换显示/隐藏
                        if double_alt::TOGGLE.swap(false, Ordering::SeqCst) {
                            let win = win.clone();
                            let handle = handle.clone();
                            let _ = handle.run_on_main_thread(move || {
                                if double_alt::WINDOW_VISIBLE.load(Ordering::SeqCst) {
                                    // 当前显示 → 隐藏
                                    double_alt::WINDOW_VISIBLE.store(false, Ordering::SeqCst);
                                    let _ = win.hide();
                                } else {
                                    // 当前隐藏 → 显示并抢焦点
                                    double_alt::WINDOW_VISIBLE.store(true, Ordering::SeqCst);
                                    let _ = win.show();

                                    // 抢焦点必须在本闭包（主线程）内直接调用，
                                    // 因为 SetFocus/SetActiveWindow/SetForegroundWindow
                                    // 都要求从拥有窗口的 UI 线程执行
                                    if let Ok(hwnd) = win.hwnd() {
                                        force_foreground(hwnd.0 as isize);
                                    }
                                }
                            });
                        }
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    });
                } else {
                    eprintln!("[luma] 未找到 main 窗口");
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, _event| {});
}
