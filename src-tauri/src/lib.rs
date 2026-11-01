use std::collections::HashSet;
use std::path::PathBuf;

mod catalog;
mod config;
mod diagnostics;
mod discovery;
#[cfg(windows)]
mod icon_resource;
mod icons;
mod launch;
mod portable;
mod runtime;
mod shortcuts;
mod watcher;

mod model;
#[cfg(windows)]
mod packaged;
use model::AppEntry;

/// 扫描应用：开始菜单 .lnk + 注册表 App Paths
#[tauri::command]
async fn scan_apps(force: Option<bool>) -> Result<catalog::ScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || catalog::scan(force.unwrap_or(false)))
        .await
        .map_err(|e| format!("扫描应用失败: {}", e))
}

fn scan_source(source: usize) -> Vec<AppEntry> {
    let mut apps = Vec::new();
    let mut seen = HashSet::new();
    if source == 0 {
        shortcuts::scan(&mut apps, &mut seen);
    }
    if source == 1 {
        discovery::scan_app_paths(&mut apps, &mut seen);

        // 3. 用户级和系统级安装记录（含没有快捷方式的应用）
        discovery::scan_installed_apps(&mut apps, &mut seen);
        #[cfg(windows)]
        packaged::scan(&mut apps, &mut seen);
        #[cfg(windows)]
        discovery::scan_file_associations(&mut apps, &mut seen);
    }
    if source == 2 {
        scan_game_dirs(&mut apps, &mut seen);
        portable::scan(&mut apps, &mut seen);
    }

    // 保留自动分类，前端根据配置映射显示手动分类，避免分类变更触发磁盘扫描。

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

/// 扫描游戏目录（绿色版游戏等），递归找 exe 并作为应用加入。
/// 目录来源：常见游戏目录 + 用户配置的 extra_dirs。
fn scan_game_dirs(apps: &mut Vec<AppEntry>, seen: &mut HashSet<String>) {
    for root in game_roots() {
        scan_dir_exes(&root, 8, apps, seen);
    }
}

fn game_roots() -> Vec<PathBuf> {
    // 常见游戏目录（各盘符根目录下）
    let mut roots: Vec<PathBuf> = Vec::new();

    // 1. 用户配置的额外目录
    for dir in &config::get().extra_dirs {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            roots.push(p);
        }
    }

    // 2. 常见游戏目录：各盘符的 game / Games / 游戏 目录
    for drive in b'A'..=b'Z' {
        let letter = drive as char;
        let base = format!("{}:\\", letter);
        for name in ["game", "Games", "Game", "游戏"] {
            let p = PathBuf::from(&base).join(name);
            if p.is_dir() {
                roots.push(p);
            }
        }
    }

    // 去重 roots
    let mut seen_roots = HashSet::new();
    roots
        .into_iter()
        .filter(|root| seen_roots.insert(root.to_string_lossy().to_lowercase()))
        .collect()
}

/// 递归扫描目录下的 exe（限制深度），加入应用列表
fn scan_dir_exes(dir: &PathBuf, depth: u32, apps: &mut Vec<AppEntry>, seen: &mut HashSet<String>) {
    if depth == 0 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(error) => {
            diagnostics::record("扫描", &dir.to_string_lossy(), None, error.to_string());
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // 跳过明显的系统/无关目录，避免误扫
            let dir_name = entry.file_name().to_string_lossy().to_lowercase();
            if dir_name == "lib" || dir_name == "node_modules" {
                continue;
            }
            scan_dir_exes(&path, depth - 1, apps, seen);
        } else if path.extension().map(|e| e.to_string_lossy().to_lowercase())
            == Some("exe".to_string())
        {
            let exe_name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            // 跳过明显的工具类 exe（安装器、卸载器、启动器辅助等）
            if is_noise_exe(&exe_name) {
                diagnostics::record(
                    "过滤",
                    &path.to_string_lossy(),
                    None,
                    "安装、卸载或辅助程序",
                );
                continue;
            }
            let full = path.to_string_lossy().to_string();
            // 用版本资源 FileDescription 取友好名，回退文件名（并清理 UE4 后缀）
            #[cfg(windows)]
            let name = friendly_name(&full).unwrap_or_else(|| clean_game_name(&exe_name));
            #[cfg(not(windows))]
            let name = clean_game_name(&exe_name);

            let mut app = AppEntry::new(name, full, "目录扫描", dir.to_string_lossy().to_string());
            app.category = "游戏".into();
            if seen.insert(app.id.clone()) {
                apps.push(app);
            }
        }
    }
}

/// 清理游戏 exe 文件名中的引擎后缀，得到干净的显示名。
/// 如 "BeastOfReincarnation-Win64-Shipping" → "BeastOfReincarnation"
fn clean_game_name(exe_name: &str) -> String {
    let mut name = exe_name.to_string();
    // UE4 常见后缀
    for suffix in [
        "-win64-shipping",
        "-win64",
        "-shipping",
        "-win32-shipping",
        "-win32",
        "-windowsnoeditor",
    ] {
        if let Some(idx) = name.to_lowercase().rfind(suffix) {
            name.truncate(idx);
            break;
        }
    }
    name
}

/// 判断 exe 文件名是否是"工具类"噪音（安装器、卸载器等），应跳过
fn is_noise_exe(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("install")
        || n.contains("setup")
        || n.contains("uninstall")
        || n.contains("unins")
        || n.contains("update")
        || n.contains("patcher")
        || n.contains("config")
        || n.contains("unwise")
        || n.contains("crashreport")
        || n.contains("crash_report")
        || n.contains("unrealpak")
        || n.contains("redist")
        || n.contains("dxsetup")
        || n.contains("vc_redist")
        || n.contains("dxwebsetup")
        || n.contains("-worker")
        || n.contains("_worker")
        || n.contains("安装")
        || n.contains("卸载")
}

/// 判断一个应用是否为"噪音"（卸载程序、命令行工具、帮助文档、修复工具等）。
/// 返回 Some(原因) 表示应被自动过滤。
fn classify_noise(name: &str, path: &str) -> Option<String> {
    let name_lower = name.to_lowercase();
    let path_lower = path.to_lowercase();
    let stem = std::path::Path::new(path)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    if [
        "anticheatexpert",
        "easyanticheat",
        "battleye",
        "crashpad_handler",
    ]
    .contains(&stem.as_str())
        || ["anticheatexpert", "easyanticheat", "battleye"].contains(&name_lower.trim())
        || stem.starts_with("crashreport")
        || stem.ends_with("_helper")
        || stem.ends_with("-helper")
    {
        return Some("后台辅助组件".into());
    }
    if name_lower.contains("redistributable")
        || name_lower.contains(".net host")
        || name_lower.contains("desktop runtime")
    {
        return Some("运行库组件".into());
    }
    if ["官方网站", "意见反馈", "打字入门", "皮肤下载", "细胞词库"].contains(&name_lower.trim())
        && [".url", ".html", ".htm"]
            .iter()
            .any(|extension| path_lower.ends_with(extension))
    {
        return Some("附属网页或文档".into());
    }

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

#[cfg(test)]
mod noise_tests {
    #[test]
    fn recognizes_component_display_names_without_hiding_real_launchers() {
        assert_eq!(
            super::classify_noise("AntiCheatExpert", r"C:\Games\ACE-Setup64.exe").as_deref(),
            Some("后台辅助组件")
        );
        assert_eq!(
            super::classify_noise("官方网站", r"C:\Apps\Sogou Pinyin Ime.url").as_deref(),
            Some("附属网页或文档")
        );
        assert!(super::classify_noise("官方网站", r"C:\Apps\SiteBuilder.exe").is_none());
        assert!(super::classify_noise("WeGame", r"C:\WeGame\WeGameLauncher.exe").is_none());
    }
}

/// 自动分类：根据应用名和路径判断所属分类。
/// 返回分类名字符串（中文），用于侧边栏分组。
fn auto_category(name: &str, path: &str) -> String {
    let name_lower = name.to_lowercase();
    let path_lower = path.to_lowercase();

    // 游戏特征：路径含 game/steam/epic 等，或名字含游戏相关词
    if path_lower.contains("\\game")
        || path_lower.contains("\\games")
        || path_lower.contains("steam")
        || path_lower.contains("\\epic games")
        || path_lower.contains("\\gog games")
        || path_lower.contains("wegame")
        || name_lower.contains("game")
        || name_lower.contains("游戏")
    {
        return "游戏".to_string();
    }

    // 开发工具
    if name_lower.contains("visual studio")
        || name_lower.starts_with("trae")
        || name_lower.contains("code")
        || name_lower.contains("jetbrains")
        || name_lower.contains("intellij")
        || name_lower.contains("pycharm")
        || name_lower.contains("webstorm")
        || name_lower.contains("goland")
        || name_lower.contains("clion")
        || name_lower.contains("rider")
        || name_lower.contains("android studio")
        || name_lower.contains("git")
        || name_lower.contains("node")
        || name_lower.contains("python")
        || name_lower.contains("rust")
        || name_lower.contains("golang")
        || name_lower.contains("terminal")
        || name_lower.contains("powershell")
        || name_lower.contains("cmd")
        || name_lower.contains("docker")
        || name_lower.contains("数据库")
        || name_lower.contains("database")
        || name_lower.contains("sql")
        || name_lower.contains("postman")
        || name_lower.contains("开发")
    {
        return "开发".to_string();
    }

    // 办公
    if name_lower.contains("word")
        || name_lower.contains("excel")
        || name_lower.contains("powerpoint")
        || name_lower.contains("outlook")
        || name_lower.contains("office")
        || name_lower.contains("wps")
        || name_lower.contains("pdf")
        || name_lower.contains("邮箱")
        || name_lower.contains("mail")
        || name_lower.contains("会议")
        || name_lower.contains("钉钉")
        || name_lower.contains("企业微信")
        || name_lower.contains("飞书")
        || name_lower.contains("notion")
        || name_lower.contains("笔记")
        || name_lower.contains("待办")
        || name_lower.contains("日历")
        || name_lower.contains("办公")
    {
        return "办公".to_string();
    }

    // 媒体 / 影音
    if name_lower.contains("player")
        || name_lower.contains("播放器")
        || name_lower.contains("music")
        || name_lower.contains("音乐")
        || name_lower.contains("video")
        || name_lower.contains("视频")
        || name_lower.contains("photo")
        || name_lower.contains("图片")
        || name_lower.contains("图像")
        || name_lower.contains("剪辑")
        || name_lower.contains("photoshop")
        || name_lower.contains("premiere")
        || name_lower.contains("剪映")
        || name_lower.contains("网易云")
        || name_lower.contains("qq音乐")
        || name_lower.contains("bilibili")
        || name_lower.contains("爱奇艺")
        || name_lower.contains("腾讯视频")
        || name_lower.contains("优酷")
        || name_lower.contains("potplayer")
        || name_lower.contains("qq影音")
    {
        return "媒体".to_string();
    }

    // 社交 / 通讯
    if name_lower.contains("微信")
        || name_lower.contains("wechat")
        || name_lower.contains("qq")
        || name_lower.contains("telegram")
        || name_lower.contains("discord")
        || name_lower.contains("whatsapp")
        || name_lower.contains("skype")
        || name_lower.contains("微博")
        || name_lower.contains("社交")
        || name_lower.contains("聊天")
    {
        return "社交".to_string();
    }

    // 浏览器
    if name_lower.contains("chrome")
        || name_lower.contains("edge")
        || name_lower.contains("firefox")
        || name_lower.contains("浏览器")
        || name_lower.contains("browser")
    {
        return "浏览器".to_string();
    }

    // 系统工具（默认兜底）
    "其他".to_string()
}

/// 用 parselnk 解析 .lnk 快捷方式
fn parse_lnk(lnk_path: &PathBuf) -> Option<AppEntry> {
    let name = lnk_path.file_stem()?.to_string_lossy().to_string();

    let bytes = std::fs::read(lnk_path).ok()?;
    let shortcut = parselnk::Lnk::try_from(bytes.as_slice()).ok()?;

    // 解析真正的目标路径：优先 link_info 里的完整路径，再回退 relative_path
    let target_str = resolve_lnk_target(&shortcut, lnk_path)?;

    let mut app = AppEntry::new(
        name,
        target_str,
        "开始菜单",
        lnk_path.to_string_lossy().to_string(),
    );
    app.shortcut_path = Some(lnk_path.to_string_lossy().to_string());
    app.arguments = shortcut
        .arguments()
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    app.working_dir = shortcut
        .working_dir()
        .map(|p| p.to_string_lossy().trim_end_matches('\0').to_string());
    app.update_identity();
    Some(app)
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
                let desc = String::from_utf16_lossy(slice)
                    .trim_end_matches('\0')
                    .trim()
                    .to_string();
                if !desc.is_empty() {
                    return Some(desc);
                }
            }
        }

        None
    }
}

#[cfg(not(windows))]
fn friendly_name(_path: &str) -> Option<String> {
    None
}

/// 启动应用（可选管理员权限）
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
fn force_foreground(hwnd: isize) {
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT, SendInput, SetActiveWindow, SetFocus,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE,
        SetForegroundWindow, SetWindowPos,
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
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOREDIRECTIONBITMAP,
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
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                ex_style & !(WS_EX_NOREDIRECTIONBITMAP as isize),
            );
        }
    }
}

#[cfg(not(windows))]
fn disable_window_corner_and_shadow(_hwnd: isize) {}

// ==================== 唤醒快捷键（低级键盘钩子）====================

#[cfg(windows)]
mod double_alt {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
        UnregisterHotKey, VK_ESCAPE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG,
        RegisterClassW, WM_HOTKEY, WNDCLASSW,
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
pub fn run() {
    let app = tauri::Builder::default()
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
            launch::launch_app,
            icons::get_icon,
            hide_window,
            config::get_config,
            config::set_config,
            config::set_autostart,
            config::set_show_all,
            config::set_extra_dirs,
            config::set_app_category,
            config::set_app_pinned,
            config::set_app_hidden,
            diagnostics::get_diagnostics,
            runtime::quit_app,
            runtime::restart_app,
            config::add_custom_category,
            config::remove_custom_category
        ])
        .setup(|app| {
            // 初始化配置（启动时加载一次）
            config::init();
            runtime::setup(app)?;
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
                                            double_alt::WINDOW_VISIBLE
                                                .store(false, Ordering::SeqCst);
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
