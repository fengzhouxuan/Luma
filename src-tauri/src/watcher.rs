//! 系统变更通知只使相关来源失效；前端收到去抖事件后按来源增量扫描。

#[cfg(windows)]
mod windows {
    use crate::{catalog, diagnostics};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use tauri::Emitter;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
        Storage::FileSystem::{
            FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
            FILE_NOTIFY_CHANGE_LAST_WRITE, FindCloseChangeNotification,
            FindFirstChangeNotificationW, FindNextChangeNotification,
        },
        System::{
            Registry::{
                REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME, RegNotifyChangeKeyValue,
            },
            Threading::{CreateEventW, WaitForSingleObject},
        },
    };
    use winreg::{
        RegKey,
        enums::{
            HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        },
    };

    struct DirectoryWatch {
        handle: HANDLE,
        source: usize,
        active: bool,
    }
    impl Drop for DirectoryWatch {
        fn drop(&mut self) {
            unsafe {
                FindCloseChangeNotification(self.handle);
            }
        }
    }
    impl DirectoryWatch {
        fn new(path: &Path, source: usize) -> Option<Self> {
            let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let flags = FILE_NOTIFY_CHANGE_FILE_NAME
                | FILE_NOTIFY_CHANGE_DIR_NAME
                | if source == 0 {
                    FILE_NOTIFY_CHANGE_LAST_WRITE
                } else {
                    0
                };
            let handle = unsafe { FindFirstChangeNotificationW(wide.as_ptr(), 1, flags) };
            if handle == INVALID_HANDLE_VALUE || handle.is_null() {
                diagnostics::record(
                    "监听",
                    &path.to_string_lossy(),
                    None,
                    "目录监听不可用，使用定时扫描兜底",
                );
                None
            } else {
                Some(Self {
                    handle,
                    source,
                    active: true,
                })
            }
        }
        fn changed(&mut self) -> bool {
            if !self.active || unsafe { WaitForSingleObject(self.handle, 0) } != WAIT_OBJECT_0 {
                return false;
            }
            self.active = unsafe { FindNextChangeNotification(self.handle) } != 0;
            true
        }
    }

    struct RegistryWatch {
        key: RegKey,
        event: HANDLE,
    }
    impl Drop for RegistryWatch {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.event);
            }
        }
    }
    impl RegistryWatch {
        fn new(key: RegKey) -> Option<Self> {
            let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
            if event.is_null() {
                return None;
            }
            let watch = Self { key, event };
            if watch.arm() { Some(watch) } else { None }
        }
        fn arm(&self) -> bool {
            unsafe {
                RegNotifyChangeKeyValue(
                    self.key.raw_handle() as _,
                    1,
                    REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET,
                    self.event,
                    1,
                ) == 0
            }
        }
        fn changed(&self) -> bool {
            if unsafe { WaitForSingleObject(self.event, 0) } != WAIT_OBJECT_0 {
                return false;
            }
            self.arm();
            true
        }
    }

    fn registry_watches() -> Vec<RegistryWatch> {
        let mut watches = Vec::new();
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
                for path in [
                    r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
                    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
                    r"SOFTWARE\Classes\Applications",
                    r"SOFTWARE\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages",
                    r"SOFTWARE\Classes\ActivatableClasses\Package",
                ] {
                    if let Ok(key) =
                        RegKey::predef(hive).open_subkey_with_flags(&path, KEY_READ | view)
                    {
                        if let Some(watch) = RegistryWatch::new(key) {
                            watches.push(watch);
                        }
                    }
                }
            }
        }
        watches
    }

    fn directory_watches() -> Vec<DirectoryWatch> {
        let mut roots: Vec<(PathBuf, usize)> = crate::shortcuts::roots()
            .into_iter()
            .map(|(path, _)| (path, 0))
            .collect();
        roots.extend(crate::game_roots().into_iter().map(|path| (path, 2)));
        roots.extend(
            crate::portable::roots()
                .into_iter()
                .map(|(path, _)| (path, 2)),
        );
        roots
            .into_iter()
            .filter(|(path, _)| path.is_dir())
            .filter_map(|(path, source)| DirectoryWatch::new(&path, source))
            .collect()
    }

    pub fn start(app: tauri::AppHandle) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("luma-app-watcher".into())
            .spawn(move || {
                let mut directories = directory_watches();
                let mut registry = registry_watches();
                catalog::watched(directories.len(), registry.len());
                let mut refreshed = Instant::now();
                let mut pending: Option<(Instant, Instant)> = None;
                let mut fallback_requested = false;
                loop {
                    let mut changed = false;
                    if catalog::full_due() {
                        if !fallback_requested {
                            for source in 0..3 {
                                catalog::invalidate(source);
                            }
                            changed = true;
                            fallback_requested = true;
                        }
                    } else {
                        fallback_requested = false;
                    }
                    for watch in &mut directories {
                        if watch.changed() {
                            catalog::invalidate(watch.source);
                            changed = true;
                        }
                    }
                    for watch in &registry {
                        if watch.changed() {
                            catalog::invalidate(1);
                            changed = true;
                        }
                    }
                    if changed {
                        let now = Instant::now();
                        if let Some((_, last)) = &mut pending {
                            *last = now;
                        } else {
                            pending = Some((now, now));
                        }
                    }
                    if pending.is_some_and(|(first, last)| {
                        last.elapsed() >= Duration::from_millis(500)
                            || first.elapsed() >= Duration::from_secs(2)
                    }) {
                        let _ = app.emit("apps-changed", ());
                        pending = None;
                    }
                    // 新建目录、重建注册表键以及设置变更都能重新注册监听。
                    if refreshed.elapsed() >= Duration::from_secs(30) {
                        directories = directory_watches();
                        registry = registry_watches();
                        catalog::watched(directories.len(), registry.len());
                        refreshed = Instant::now();
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            })?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn registry_values_are_observed_and_rearmed() {
            let name = format!(
                r"Software\LumaWatcherTest-{}-{}",
                std::process::id(),
                diagnostics::now()
            );
            let root = RegKey::predef(HKEY_CURRENT_USER);
            let (key, _) = root.create_subkey(&name).unwrap();
            struct Cleanup(String);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey(&self.0);
                }
            }
            let _cleanup = Cleanup(name.clone());
            let watch = RegistryWatch::new(key).unwrap();
            let writer = root
                .open_subkey_with_flags(&name, winreg::enums::KEY_WRITE)
                .unwrap();
            for value in [1u32, 2u32] {
                writer.set_value("fixture", &value).unwrap();
                let deadline = Instant::now() + Duration::from_secs(3);
                while !watch.changed() {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
        #[test]
        fn directory_changes_are_observed_and_rearmed() {
            let root = std::env::temp_dir().join(format!(
                "luma-watch-{}-{}",
                std::process::id(),
                diagnostics::now()
            ));
            std::fs::create_dir(&root).unwrap();
            let mut watch = DirectoryWatch::new(&root, 0).unwrap();
            for file in ["one.lnk", "two.lnk"] {
                std::fs::write(root.join(file), b"fixture").unwrap();
                let deadline = Instant::now() + Duration::from_secs(3);
                while !watch.changed() {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            drop(watch);
            for file in ["one.lnk", "two.lnk"] {
                std::fs::remove_file(root.join(file)).unwrap();
            }
            std::fs::remove_dir(root).unwrap();
        }
    }
}

#[cfg(windows)]
pub use windows::start;
#[cfg(not(windows))]
pub fn start(_app: tauri::AppHandle) -> std::io::Result<()> {
    Ok(())
}
