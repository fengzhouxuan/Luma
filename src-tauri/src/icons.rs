//! Shell 图标只在专用 STA 线程提取；缓存同时跟踪快捷方式、目标和图标资源。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime};

const FAILURE_TTL: Duration = Duration::from_secs(30);
const SUCCESS_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const CACHE_ENTRIES: usize = 512;
const CACHE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    path: String,
    modified: Option<SystemTime>,
    len: Option<u64>,
}

fn shell_path(path: &str) -> String {
    let path = Path::new(path.trim_matches('"'));
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    normalize_shell_path(&canonical.to_string_lossy())
}

fn normalize_shell_path(path: &str) -> String {
    if path
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
    {
        return format!(r"\\{}", &path[8..]);
    }
    if let Some(local) = path.strip_prefix(r"\\?\") {
        // 保留 Volume GUID 等没有普通 DOS 路径形式的命名空间。
        if local.as_bytes().get(1) == Some(&b':') {
            return local.to_string();
        }
    }
    path.to_string()
}

fn file_stamp(path: &str) -> FileStamp {
    let metadata = std::fs::metadata(path).ok();
    FileStamp {
        path: path.to_string(),
        modified: metadata.as_ref().and_then(|m| m.modified().ok()),
        len: metadata.map(|m| m.len()),
    }
}

fn expand_environment(path: &str) -> String {
    let mut result = String::new();
    let mut rest = path;
    while let Some(start) = rest.find('%') {
        result.push_str(&rest[..start]);
        let variable = &rest[start + 1..];
        let Some(end) = variable.find('%') else {
            result.push_str(&rest[start..]);
            return result;
        };
        match std::env::var(&variable[..end]) {
            Ok(value) => result.push_str(&value),
            Err(_) => result.push_str(&rest[start..start + end + 2]),
        }
        rest = &variable[end + 1..];
    }
    result.push_str(rest);
    result
}

struct IconResource {
    path: String,
    index: i32,
}

fn shortcut_resource(shortcut_path: &str) -> Option<IconResource> {
    let bytes = std::fs::read(shortcut_path).ok()?;
    let shortcut = parselnk::Lnk::try_from(bytes.as_slice()).ok()?;
    let location = shortcut.string_data.icon_location.clone().or_else(|| {
        if shortcut.header.icon_index != 0 {
            crate::resolve_lnk_target(&shortcut, &PathBuf::from(shortcut_path)).map(PathBuf::from)
        } else {
            None
        }
    })?;
    let location = expand_environment(location.to_string_lossy().trim_matches('"'));
    if location.is_empty() {
        return None;
    }
    let resource = PathBuf::from(location);
    let resource = if resource.is_absolute() {
        resource
    } else {
        let base = shortcut
            .working_dir()
            .or_else(|| Path::new(shortcut_path).parent().map(Path::to_path_buf))?;
        let candidate = base.join(&resource);
        if candidate.is_file() {
            candidate
        } else {
            let system = std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .map(|root| root.join("System32").join(&resource));
            system.filter(|path| path.is_file()).unwrap_or(candidate)
        }
    };
    Some(IconResource {
        path: shell_path(&resource.to_string_lossy()),
        index: shortcut.header.icon_index as i32,
    })
}

struct CachedIcon {
    fingerprint: Vec<FileStamp>,
    icon: Option<String>,
    created: Instant,
    used: Instant,
    ttl: Duration,
}

struct IconCache {
    entries: HashMap<String, CachedIcon>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl IconCache {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            max_entries,
            max_bytes,
        }
    }

    fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key) {
            self.bytes -= entry.icon.as_ref().map_or(0, String::len);
        }
    }

    fn get(
        &mut self,
        key: &str,
        fingerprint: &[FileStamp],
        now: Instant,
    ) -> Option<Option<String>> {
        let entry = self.entries.get(key)?;
        if entry.fingerprint != fingerprint || now.duration_since(entry.created) >= entry.ttl {
            self.remove(key);
            return None;
        }
        let entry = self.entries.get_mut(key)?;
        entry.used = now;
        Some(entry.icon.clone())
    }

    fn put(
        &mut self,
        key: String,
        fingerprint: Vec<FileStamp>,
        icon: Option<String>,
        retry: bool,
        now: Instant,
    ) {
        self.remove(&key);
        self.bytes += icon.as_ref().map_or(0, String::len);
        self.entries.insert(
            key,
            CachedIcon {
                fingerprint,
                icon,
                created: now,
                used: now,
                ttl: if retry { FAILURE_TTL } else { SUCCESS_TTL },
            },
        );
        while self.entries.len() > self.max_entries || self.bytes > self.max_bytes {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest {
                self.remove(&key);
            } else {
                break;
            }
        }
    }
}

struct IconRequest {
    path: String,
    shortcut_path: Option<String>,
    queued: Instant,
    reply: mpsc::SyncSender<Option<String>>,
}

static WORKER: OnceLock<Result<mpsc::SyncSender<IconRequest>, String>> = OnceLock::new();

#[tauri::command]
pub async fn get_icon(path: String, shortcut_path: Option<String>) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = shortcut_path.as_deref().unwrap_or(&path).to_string();
        let worker = WORKER.get_or_init(start_worker);
        let sender = match worker {
            Ok(sender) => sender,
            Err(error) => {
                eprintln!("[luma icons] {}", error);
                crate::diagnostics::record("图标", &source, None, error.clone());
                return None;
            }
        };
        let (reply, receiver) = mpsc::sync_channel(1);
        if let Err(error) = sender.try_send(IconRequest {
            path,
            shortcut_path,
            queued: Instant::now(),
            reply,
        }) {
            eprintln!("[luma icons] 图标队列不可用: {}", error);
            crate::diagnostics::record("图标", &source, None, format!("图标队列不可用: {error}"));
            return None;
        }
        match receiver.recv_timeout(REQUEST_TIMEOUT) {
            Ok(icon) => icon,
            Err(error) => {
                eprintln!("[luma icons] 等待图标失败: {}", error);
                crate::diagnostics::record("图标", &source, None, format!("等待图标失败: {error}"));
                None
            }
        }
    })
    .await
    .unwrap_or_else(|error| {
        eprintln!("[luma icons] 图标任务失败: {}", error);
        None
    })
}

fn start_worker() -> Result<mpsc::SyncSender<IconRequest>, String> {
    let (sender, receiver) = mpsc::sync_channel::<IconRequest>(32);
    std::thread::Builder::new()
        .name("luma-icons-sta".into())
        .spawn(move || {
            #[cfg(windows)]
            let _com = match ComApartment::enter() {
                Ok(com) => com,
                Err(error) => {
                    eprintln!("[luma icons] {}", error);
                    return;
                }
            };
            let mut cache = IconCache::new(CACHE_ENTRIES, CACHE_BYTES);
            loop {
                #[cfg(windows)]
                pump_messages();
                let request = match receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(request) => request,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                // 调用方已超时的排队任务不再触发磁盘或 Shell 访问。
                if request.queued.elapsed() >= REQUEST_TIMEOUT {
                    continue;
                }
                let icon = cached_icon(&mut cache, &request.path, request.shortcut_path.as_deref());
                let _ = request.reply.send(icon);
            }
        })
        .map_err(|e| format!("创建图标线程失败: {}", e))?;
    Ok(sender)
}

fn cached_icon(cache: &mut IconCache, path: &str, shortcut_path: Option<&str>) -> Option<String> {
    let target = shell_path(path);
    let shortcut = shortcut_path.map(shell_path);
    let key = format!("{}\0{}", target, shortcut.as_deref().unwrap_or(""));
    let mut fingerprint = vec![file_stamp(&target)];
    let resource = shortcut.as_deref().and_then(shortcut_resource);
    if let Some(shortcut) = &shortcut {
        fingerprint.push(file_stamp(shortcut));
    }
    if let Some(resource) = &resource {
        let mut stamp = file_stamp(&resource.path);
        stamp.path.push_str(&format!("\0{}", resource.index));
        fingerprint.push(stamp);
    }
    let now = Instant::now();
    if let Some(icon) = cache.get(&key, &fingerprint, now) {
        return icon;
    }
    let mut retry = false;
    let mut icon = None;
    let mut errors = Vec::new();
    if let Some(shortcut) = shortcut {
        let result = match resource {
            // 显式资源直接读取索引，避免依赖 Shell 图标缓存的刷新时机。
            #[cfg(windows)]
            Some(resource) => crate::icon_resource::extract(&resource.path, resource.index),
            _ => extract_icon(&shortcut),
        };
        match result {
            Ok(value) => icon = Some(value),
            Err(error) => {
                eprintln!("[luma icons] {}", error);
                errors.push(error);
                retry = true;
            }
        }
    }
    if icon.is_none() {
        match extract_icon(&target) {
            Ok(value) => icon = Some(value),
            Err(error) => {
                eprintln!("[luma icons] {}", error);
                errors.push(error);
                retry = true;
            }
        }
    }
    let message = if icon.is_some() {
        if retry {
            format!("已回退到 EXE；{}", errors.join("；"))
        } else if shortcut_path.is_some() {
            "已加载快捷方式图标".into()
        } else {
            "已加载程序图标".into()
        }
    } else {
        format!("图标提取失败；{}", errors.join("；"))
    };
    crate::diagnostics::record("图标", shortcut_path.unwrap_or(path), None, message);
    // 虚拟 Shell 项没有文件时间戳，缩短缓存，避免应用更新后长期使用旧图标。
    let short_lived = retry || target.starts_with("shell:AppsFolder\\");
    cache.put(key, fingerprint, icon.clone(), short_lived, now);
    icon
}

#[cfg(windows)]
fn extract_icon(path: &str) -> Result<String, String> {
    if crate::packaged::is_shell_item(path) {
        return crate::packaged::icon(path);
    }
    if !Path::new(path).is_file() {
        return Err(format!("图标来源文件不存在或不可读: {path}"));
    }
    windows_icons::get_icon_base64_by_path_with_size(path, windows_icons::IconSize::Large)
        .map(|b64| format!("data:image/png;base64,{}", b64))
        .map_err(|e| format!("提取图标失败 ({path}): {e}"))
}

#[cfg(not(windows))]
fn extract_icon(_path: &str) -> Result<String, String> {
    Err("当前平台不支持 Shell 图标".into())
}

#[cfg(windows)]
struct ComApartment;

#[cfg(windows)]
impl ComApartment {
    fn enter() -> Result<Self, String> {
        use windows_sys::Win32::System::Com::{
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
        };
        let hr = unsafe {
            CoInitializeEx(
                std::ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        };
        if hr < 0 {
            Err(format!("初始化图标 STA 失败: {hr:#x}"))
        } else {
            Ok(Self)
        }
    }
}

#[cfg(windows)]
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
    }
}

#[cfg(windows)]
fn pump_messages() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(version: u64) -> Vec<FileStamp> {
        vec![FileStamp {
            path: "app.exe".into(),
            modified: None,
            len: Some(version),
        }]
    }

    #[test]
    fn normalizes_local_and_unc_without_breaking_device_paths() {
        assert_eq!(
            normalize_shell_path(r"\\?\C:\apps\app.exe"),
            r"C:\apps\app.exe"
        );
        assert_eq!(
            normalize_shell_path(r"\\?\UNC\server\share\app.exe"),
            r"\\server\share\app.exe"
        );
        assert_eq!(
            normalize_shell_path(r"\\server\share\app.exe"),
            r"\\server\share\app.exe"
        );
        assert_eq!(
            normalize_shell_path(r"\\?\Volume{abc}\app.exe"),
            r"\\?\Volume{abc}\app.exe"
        );
    }

    #[test]
    fn retries_failures_and_degraded_fallbacks_after_thirty_seconds() {
        let mut cache = IconCache::new(10, 1024);
        let now = Instant::now();
        for icon in [None, Some("fallback".into())] {
            cache.put("app".into(), stamp(1), icon.clone(), true, now);
            assert_eq!(
                cache.get("app", &stamp(1), now + Duration::from_secs(29)),
                Some(icon)
            );
            assert_eq!(
                cache.get("app", &stamp(1), now + Duration::from_secs(30)),
                None
            );
        }
    }

    #[test]
    fn invalidates_changed_dependencies_without_accumulating_old_versions() {
        let mut cache = IconCache::new(10, 1024);
        let now = Instant::now();
        cache.put("app".into(), stamp(1), Some("first".into()), false, now);
        assert_eq!(cache.get("app", &stamp(2), now), None);
        cache.put("app".into(), stamp(2), Some("new".into()), false, now);
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.bytes, 3);
    }

    #[test]
    fn evicts_least_recently_used_entries_and_enforces_byte_limit() {
        let mut cache = IconCache::new(2, 6);
        let now = Instant::now();
        cache.put("a".into(), stamp(1), Some("aaa".into()), false, now);
        cache.put("b".into(), stamp(1), Some("bbb".into()), false, now);
        cache.get("a", &stamp(1), now + Duration::from_secs(1));
        cache.put(
            "c".into(),
            stamp(1),
            Some("ccc".into()),
            false,
            now + Duration::from_secs(2),
        );
        assert!(cache.entries.contains_key("a"));
        assert!(!cache.entries.contains_key("b"));
        assert!(cache.bytes <= 6);
        cache.put(
            "large".into(),
            stamp(1),
            Some("too-large".into()),
            false,
            now + Duration::from_secs(3),
        );
        assert!(cache.bytes <= 6);
    }

    #[cfg(windows)]
    #[test]
    fn shell_honors_shortcut_resource_indexes_and_missing_shortcut_falls_back_to_exe() {
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                for name in ["first.lnk", "second.lnk", "ico.lnk", "custom.ico"] {
                    let _ = std::fs::remove_file(self.0.join(name));
                }
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fixture = Fixture(
            std::env::temp_dir().join(format!("luma-icon-test-{}-{nonce}", std::process::id())),
        );
        std::fs::create_dir(&fixture.0).unwrap();
        fn ico(color: [u8; 4]) -> Vec<u8> {
            let image = image::RgbaImage::from_fn(32, 32, |x, y| {
                image::Rgba(if (8..24).contains(&x) && (8..24).contains(&y) {
                    color
                } else {
                    [0, 0, 0, 0]
                })
            });
            let mut png = std::io::Cursor::new(Vec::new());
            image.write_to(&mut png, image::ImageFormat::Png).unwrap();
            let png = png.into_inner();
            let mut icon = vec![0, 0, 1, 0, 1, 0, 32, 32, 0, 0, 1, 0, 32, 0];
            icon.extend_from_slice(&(png.len() as u32).to_le_bytes());
            icon.extend_from_slice(&22u32.to_le_bytes());
            icon.extend_from_slice(&png);
            icon
        }
        std::fs::write(fixture.0.join("custom.ico"), ico([255, 0, 0, 128])).unwrap();
        let windows = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
        let target = windows
            .join("System32/cmd.exe")
            .to_string_lossy()
            .to_string();
        let resource = windows.join("System32/shell32.dll");
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                r#"
$shell = New-Object -ComObject WScript.Shell
$first = $shell.CreateShortcut((Join-Path $env:LUMA_ICON_FIXTURE 'first.lnk'))
$first.TargetPath = $env:LUMA_ICON_TARGET
$first.IconLocation = $env:LUMA_ICON_RESOURCE + ',3'
$first.Save()
$second = $shell.CreateShortcut((Join-Path $env:LUMA_ICON_FIXTURE 'second.lnk'))
$second.TargetPath = $env:LUMA_ICON_TARGET
$second.IconLocation = $env:LUMA_ICON_RESOURCE + ',0'
$second.Save()
$ico = $shell.CreateShortcut((Join-Path $env:LUMA_ICON_FIXTURE 'ico.lnk'))
$ico.TargetPath = $env:LUMA_ICON_TARGET
$ico.IconLocation = (Join-Path $env:LUMA_ICON_FIXTURE 'custom.ico') + ',0'
$ico.Save()
"#,
            ])
            .env("LUMA_ICON_FIXTURE", &fixture.0)
            .env("LUMA_ICON_TARGET", &target)
            .env("LUMA_ICON_RESOURCE", resource)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let _com = ComApartment::enter().unwrap();
        let mut cache = IconCache::new(10, CACHE_BYTES);
        assert_eq!(
            shortcut_resource(&fixture.0.join("first.lnk").to_string_lossy())
                .unwrap()
                .index,
            3
        );
        assert_eq!(
            shortcut_resource(&fixture.0.join("second.lnk").to_string_lossy())
                .unwrap()
                .index,
            0
        );
        let first = cached_icon(
            &mut cache,
            &target,
            Some(&fixture.0.join("first.lnk").to_string_lossy()),
        )
        .unwrap();
        let second = cached_icon(
            &mut cache,
            &target,
            Some(&fixture.0.join("second.lnk").to_string_lossy()),
        )
        .unwrap();
        assert!(
            first != second,
            "同一 EXE 的两个快捷方式应使用各自的资源索引"
        );
        fn decode(data: &str) -> image::RgbaImage {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.strip_prefix("data:image/png;base64,").unwrap())
                .unwrap();
            image::load_from_memory(&bytes).unwrap().to_rgba8()
        }
        let ico_path = fixture.0.join("ico.lnk").to_string_lossy().to_string();
        let red = cached_icon(&mut cache, &target, Some(&ico_path)).unwrap();
        let red_image = decode(&red);
        assert_eq!(red_image.dimensions(), (96, 96));
        assert_eq!(red_image.get_pixel(0, 0).0[3], 0);
        let center = red_image.get_pixel(48, 48).0;
        assert!(
            center[0] > 240 && center[1] < 20 && (100..160).contains(&center[3]),
            "RGBA={center:?}"
        );
        let mut green = ico([0, 255, 0, 128]);
        green.push(0); // 文件长度也变化，不依赖文件系统时间戳精度。
        std::fs::write(fixture.0.join("custom.ico"), green).unwrap();
        let green = decode(&cached_icon(&mut cache, &target, Some(&ico_path)).unwrap());
        let center = green.get_pixel(48, 48).0;
        assert!(
            center[0] < 20 && center[1] > 240,
            "自定义资源更新后必须失效: RGBA={center:?}"
        );
        let direct = cached_icon(&mut cache, &target, None).unwrap();
        let fallback = cached_icon(
            &mut cache,
            &target,
            Some(&fixture.0.join("missing.lnk").to_string_lossy()),
        )
        .unwrap();
        assert_eq!(direct, fallback);
        let reply = tauri::async_runtime::block_on(get_icon(target, None)).unwrap();
        assert_eq!(reply, direct, "专用 STA 线程应返回同样的 EXE 图标");
        use windows_sys::Win32::System::Threading::{
            GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
        };
        let resource = windows
            .join("System32/shell32.dll")
            .to_string_lossy()
            .to_string();
        crate::icon_resource::extract(&resource, 3).unwrap();
        let before = unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        for _ in 0..32 {
            crate::icon_resource::extract(&resource, 3).unwrap();
        }
        let after = unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        assert!(
            after <= before + 2,
            "GDI 句柄不应随提取次数增长: {before} -> {after}"
        );
    }
}
