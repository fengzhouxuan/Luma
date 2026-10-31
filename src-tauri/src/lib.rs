use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;

mod config;

#[derive(Serialize, Clone)]
struct AppEntry {
    name: String,
    path: String,
    /// 图标的 base64 data URL（如 "data:image/png;base64,..."），提取失败则为 None
    icon: Option<String>,
    /// 非 None 表示被自动过滤（噪音应用），值为过滤原因，前端据此提供"手动显示"入口
    hidden_reason: Option<String>,
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

/// 判断一个应用是否为"噪音"（卸载程序、命令行工具、帮助文档、修复工具等）。
/// 返回 Some(原因) 表示应被自动过滤。
fn classify_noise(name: &str, path: &str) -> Option<String> {
    let name_lower = name.to_lowercase();
    let path_lower = path.to_lowercase();

    // 卸载程序（最常见噪音）
    if name_lower.contains("uninstall")
        || name_lower.contains("卸载")
        || name_lower.contains("deinstall")
    {
        return Some("卸载程序".to_string());
    }

    // 命令行工具 / 开发者命令行（Visual Studio 等生成的一堆 prompt 快捷方式）
    if name_lower.contains("command prompt")
        || name_lower.contains("命令行")
        || name_lower.contains("cross tools")
        || name_lower.contains("native tools")
        || name_lower.contains("developer prompt")
        || name_lower.contains("x86_")
        || name_lower.contains("x64_")
    {
        return Some("命令行工具".to_string());
    }

    // 帮助 / 文档 / 说明
    if name_lower.contains("readme")
        || name_lower.contains("帮助")
        || name_lower.contains("帮助文档")
        || name_lower.contains("manual")
        || name_lower.contains("help")
        || name_lower.contains("documentation")
        || name_lower.contains("用户手册")
    {
        return Some("帮助文档".to_string());
    }

    // 修复 / 诊断工具
    if name_lower.contains("repair")
        || name_lower.contains("修复")
        || name_lower.contains("diagnostic")
        || name_lower.contains("诊断")
    {
        return Some("诊断/修复工具".to_string());
    }

    // 更新 / 升级工具
    if name_lower.contains("update")
        || name_lower.contains("更新")
        || name_lower.contains("upgrade")
    {
        return Some("更新工具".to_string());
    }

    // 路径特征：目标位于 uninstall 目录
    if path_lower.contains("\\uninstall")
        || path_lower.contains("unins")
        || path_lower.contains("unwise")
        || path_lower.contains("uninstall.exe")
    {
        return Some("卸载程序".to_string());
    }

    None
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
        name: name.clone(),
        path: target_str.clone(),
        icon: None,
        hidden_reason: classify_noise(&name, &target_str),
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
                        let hidden_reason = classify_noise(&name, &path);
                        apps.push(AppEntry {
                            name,
                            path,
                            icon: None,
                            hidden_reason,
                        });
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

// ==================== 唤醒快捷键（低级键盘钩子）====================

#[cfg(windows)]
mod double_alt {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
        MOD_SHIFT, MOD_WIN, VK_ESCAPE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, MSG,
        WNDCLASSW, HWND_MESSAGE, WM_HOTKEY,
    };

    use crate::config::Hotkey;

    /// 唤醒快捷键的注册 ID（RegisterHotKey 用它区分消息来源）
    const HOTKEY_ID: i32 = 1;

    /// 全局标志：检测到唤醒快捷键时置 true，主线程读取后复位
    pub static TOGGLE: AtomicBool = AtomicBool::new(false);

    /// 标记窗口是否处于"由我们主动显示"的状态（权威来源，避免 is_visible 状态不一致）
    pub static WINDOW_VISIBLE: AtomicBool = AtomicBool::new(false);

    /// 终极兜底：直接轮询物理键盘状态判断 Esc 是否被按下。
    /// 不依赖焦点、不依赖钩子投递，专门解决"WebView2 拥有焦点时
    /// 低级键盘钩子静默停止投递"导致的 Esc 失效问题。
    /// 返回值 true 表示 Esc 当前正被按下（最高位为 1）。
    pub fn esc_down() -> bool {
        unsafe { (GetAsyncKeyState(VK_ESCAPE as i32) & (0x8000u16 as i16)) != 0 }
    }

    /// 将配置里的修饰键名映射为 RegisterHotKey 的 MOD_* 标志位
    fn modifier_flags(mods: &[String]) -> u32 {
        let mut flags = 0u32;
        for m in mods {
            match m.as_str() {
                "ctrl" => flags |= MOD_CONTROL,
                "alt" => flags |= MOD_ALT,
                "shift" => flags |= MOD_SHIFT,
                "meta" | "win" => flags |= MOD_WIN,
                _ => {}
            }
        }
        flags
    }

    /// 将配置里的主键名映射为虚拟键码
    fn key_vk(key: &str) -> u32 {
        match key.to_ascii_lowercase().as_str() {
            "space" => 0x20,
            "enter" => 0x0D,
            "tab" => 0x09,
            "esc" => 0x1B,
            "delete" => 0x2E,
            "backspace" => 0x08,
            "left" => 0x25,
            "up" => 0x26,
            "right" => 0x27,
            "down" => 0x28,
            k if k.starts_with('f') => {
                if let Ok(n) = k[1..].parse::<u32>() {
                    if (1..=12).contains(&n) {
                        return 0x70 + n - 1;
                    }
                }
                0
            }
            k if k.len() == 1 => {
                let c = k.as_bytes()[0];
                if c.is_ascii_lowercase() {
                    (c - b'a' + 0x41) as u32
                } else if c.is_ascii_digit() {
                    c as u32
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    /// message-only 隐藏窗口的窗口过程：收到 WM_HOTKEY 时置 TOGGLE
    unsafe extern "system" fn hotkey_window_proc(
        hwnd: HWND,
        msg: u32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if msg == WM_HOTKEY && w_param as i32 == HOTKEY_ID {
            TOGGLE.store(true, Ordering::SeqCst);
            return 0;
        }
        unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) }
    }

    /// 注册唤醒快捷键，返回是否成功。
    /// 使用 RegisterHotKey（PowerToys 同款正规军方案）：
    /// 系统级注册，不受焦点/WebView2 影响，比 WH_KEYBOARD_LL 钩子稳定得多。
    pub fn register(hotkey: &Hotkey) -> Result<(), String> {
        let (mods, key) = match hotkey {
            Hotkey::Combo { modifiers, key } => (modifier_flags(modifiers), key_vk(key)),
            // 双击单键模式不再支持：RegisterHotKey 无法表达"双击"，
            // 但为兼容旧配置，回退为"该键 + Ctrl"组合。
            Hotkey::Double { key } => (MOD_CONTROL, key_vk(key)),
        };

        if key == 0 {
            return Err(format!("无法识别的快捷键主键: {:?}", hotkey));
        }

        // MOD_NOREPEAT：按住不放不重复触发，只在首次按下时触发一次
        let fs_modifiers = mods | MOD_NOREPEAT;

        // 热键注册到 message-only 窗口（HWND_MESSAGE = -3），
        // 由独立线程的消息循环接收 WM_HOTKEY。
        let hwnd = hotkey_window();
        if hwnd == 0 {
            return Err("创建热键窗口失败".to_string());
        }

        unsafe {
            // 幂等：先反注册旧热键（避免改键后重复注册失败）
            let _ = UnregisterHotKey(hwnd as _, HOTKEY_ID);
            let ok = RegisterHotKey(hwnd as _, HOTKEY_ID, fs_modifiers, key);
            if ok == 0 {
                return Err(format!(
                    "注册快捷键失败（可能被其他程序占用）: {:?}",
                    hotkey
                ));
            }
        }
        Ok(())
    }

    /// 获取（或创建）承载热键的 message-only 隐藏窗口
    fn hotkey_window() -> isize {
        static WINDOW: OnceLock<isize> = OnceLock::new();
        *WINDOW.get_or_init(|| unsafe {
            let class_name = windows_sys::core::w!("LumaHotkeyWindow");

            // 注册窗口类
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(hotkey_window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: std::ptr::null_mut(),
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name,
            };
            RegisterClassW(&wc);

            // 创建 message-only 窗口（父窗口 HWND_MESSAGE）
            let hwnd = CreateWindowExW(
                0,
                class_name,
                class_name,
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            hwnd as isize
        })
    }

    /// 启动热键消息循环线程（阻塞在 GetMessageW 里接收 WM_HOTKEY）
    pub fn start() {
        std::thread::spawn(|| unsafe {
            // 先确保窗口已创建（register 已调用过），这里再兜底创建一次
            let hwnd = hotkey_window();
            if hwnd == 0 {
                eprintln!("[luma] 热键窗口创建失败，唤醒快捷键不可用");
                return;
            }

            let mut msg: MSG = std::mem::zeroed();
            loop {
                // 阻塞等待消息；返回 0 表示收到 WM_QUIT，退出
                let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                if ret <= 0 {
                    break;
                }
                DispatchMessageW(&msg);
            }
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
        .invoke_handler(tauri::generate_handler![
            scan_apps,
            launch_app,
            get_icon,
            hide_window,
            config::get_config,
            config::set_config,
            config::set_autostart,
            config::set_show_all
        ])
        .setup(|app| {
            // 初始化配置（启动时加载一次）
            config::init();
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

                    // 注册唤醒快捷键（RegisterHotKey 正规军方案）并启动热键消息循环
                    let hotkey_cfg = crate::config::get().hotkey;
                    if let Err(e) = double_alt::register(&hotkey_cfg) {
                        eprintln!("[luma] {}", e);
                    }
                    double_alt::start();

                    // 轮询热键标志，切换到主线程执行显示/隐藏
                    // 关键：show + 抢焦点必须在主线程（UI 线程）执行，
                    // 否则 SetForegroundWindow 受前台锁限制导致焦点抢不到
                    let win = window.clone();
                    let handle = app.handle().clone();
                    std::thread::spawn(move || {
                        // Esc 物理键轮询的"上一次按下"状态，用于边沿检测（按下瞬间才触发一次）
                        let mut esc_prev_down = false;
                        loop {
                        // 终极兜底：物理键盘轮询 Esc。即使 WebView2 焦点导致
                        // 低级键盘钩子失聪、前端 keydown 收不到，这里也能捕获并隐藏。
                        // 仅在窗口"由我们主动显示"时生效，避免后台误触发。
                        if double_alt::WINDOW_VISIBLE.load(Ordering::SeqCst) {
                            let esc_now_down = double_alt::esc_down();
                            if esc_now_down && !esc_prev_down {
                                double_alt::WINDOW_VISIBLE.store(false, Ordering::SeqCst);
                                let win = win.clone();
                                let handle = handle.clone();
                                let _ = handle.run_on_main_thread(move || {
                                    let _ = win.hide();
                                });
                            }
                            esc_prev_down = esc_now_down;
                        } else {
                            esc_prev_down = false;
                        }

                        // 处理唤醒热键：切换显示/隐藏
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
                        }
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
