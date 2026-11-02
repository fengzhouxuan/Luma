//! 补充没有开始菜单快捷方式的已安装应用，不执行安装或卸载命令。

use crate::{AppEntry, friendly_name};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn expand_environment(value: &str) -> String {
    let mut result = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            result.push_str(&rest[start..]);
            return result;
        };
        result.push_str(
            &std::env::var(&after[..end])
                .unwrap_or_else(|_| rest[start..start + end + 2].to_string()),
        );
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    result
}

fn icon_file(value: &str) -> PathBuf {
    let value = value.trim().trim_end_matches('\0');
    let value = if let Some(quoted) = value.strip_prefix('"') {
        quoted.split('"').next().unwrap_or(quoted)
    } else if let Some((path, index)) = value.rsplit_once(',') {
        if index.trim().parse::<i32>().is_ok() {
            path.trim()
        } else {
            value
        }
    } else {
        value
    };
    PathBuf::from(expand_environment(value))
}

fn executable(path: &Path) -> bool {
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return false;
    }
    let name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    ![
        "unins",
        "uninstall",
        "setup",
        "update",
        "installer",
        "maintenancetool",
    ]
    .iter()
    .any(|part| name.contains(part))
        && !maintenance_description(&format!(
            "{} {}",
            crate::version_string(&path.to_string_lossy(), "FileDescription").unwrap_or_default(),
            crate::version_string(&path.to_string_lossy(), "ProductName").unwrap_or_default()
        ))
}

fn maintenance_description(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        " setup",
        " installer",
        "uninstall",
        "update and remove",
        "更新程序",
        "安装程序",
        "卸载程序",
    ]
    .iter()
    .any(|part| text.contains(part))
}

fn installation_name(name: &str) -> String {
    let name = name.trim_end_matches('\0').trim();
    if let Some((base, suffix)) = name.rsplit_once('(') {
        if ["user)", "system)"]
            .iter()
            .any(|kind| suffix.eq_ignore_ascii_case(kind))
        {
            return base.trim().to_string();
        }
    }
    name.to_string()
}

fn normalized_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn installation_target(
    name: &str,
    location: &str,
    display_icon: &str,
) -> Option<PathBuf> {
    let icon = icon_file(display_icon);
    if executable(&icon) {
        return Some(crate::app_identity::version_target(&icon).unwrap_or(icon));
    }
    // 只在已知安装布局中找同名主程序，不把组件 EXE 或卸载命令当成入口。
    let directory = PathBuf::from(expand_environment(location.trim().trim_matches('"')));
    if location.trim().is_empty() || !directory.is_dir() {
        return None;
    }
    let expected = normalized_name(&installation_name(name));
    let folder = normalized_name(&directory.file_name()?.to_string_lossy());
    let mut directories: Vec<_> = ["", "bin", "Application", "app", "current", "current/bin"]
        .iter()
        .map(|part| directory.join(part))
        .collect();
    let mut versions: Vec<_> = std::fs::read_dir(&directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("app-"))
        })
        .collect();
    let version = |path: &PathBuf| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|part| part.parse::<u64>().ok())
            .collect::<Vec<_>>()
    };
    versions.sort_by_key(|path| std::cmp::Reverse(version(path)));
    directories.extend(versions);
    for directory in directories {
        let mut candidates: Vec<_> = std::fs::read_dir(directory)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        candidates.sort();
        if let Some(target) = candidates.into_iter().find(|path| {
            let actual = normalized_name(&path.file_stem().unwrap_or_default().to_string_lossy());
            executable(path) && (actual == expected || actual == folder)
        }) {
            return Some(target);
        }
    }
    None
}

#[cfg(windows)]
pub fn scan_app_paths(apps: &mut Vec<AppEntry>, seen: &mut HashSet<String>) {
    use winreg::{
        RegKey,
        enums::{
            HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        },
    };
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
                KEY_READ | view,
            ) else {
                continue;
            };
            for key in root.enum_keys().flatten() {
                if !key.to_ascii_lowercase().ends_with(".exe") {
                    continue;
                }
                let Ok(entry) = root.open_subkey(&key) else {
                    continue;
                };
                let Ok(value) = entry.get_value::<String, _>("") else {
                    continue;
                };
                let target = icon_file(&value);
                if !executable(&target) || !crate::portable::gui_executable(&target) {
                    crate::diagnostics::record(
                        "扫描",
                        &value,
                        None,
                        "App Paths 的目标不是有效主程序",
                    );
                    continue;
                }
                let path = target.to_string_lossy().to_string();
                let name = friendly_name(&path).unwrap_or_else(|| key[..key.len() - 4].to_string());
                let scope = if hive == HKEY_CURRENT_USER {
                    "用户"
                } else {
                    "系统"
                };
                let mut app = AppEntry::new(
                    name,
                    path,
                    &format!("App Paths（{scope}）"),
                    format!("{scope}\\App Paths\\{key}"),
                );
                if crate::package_identity::internal_executable(&target) {
                    app.hidden_reason = Some("包清单未声明为应用的内部程序".into());
                }
                if seen.insert(app.id.clone()) {
                    apps.push(app);
                }
            }
        }
    }
}

fn association_target(command: &str) -> Option<PathBuf> {
    let command = expand_environment(command.trim());
    let path = if let Some(rest) = command.strip_prefix('"') {
        PathBuf::from(rest.split('"').next()?)
    } else {
        let lower = command.to_ascii_lowercase();
        let end = lower
            .match_indices(".exe")
            .map(|(offset, _)| offset + 4)
            .find(|end| {
                command.get(*end..).is_some_and(|suffix| {
                    suffix.is_empty() || suffix.starts_with(char::is_whitespace)
                })
            })?;
        PathBuf::from(&command[..end])
    };
    executable(&path).then_some(path)
}

#[cfg(windows)]
pub fn scan_file_associations(apps: &mut Vec<AppEntry>, seen: &mut HashSet<String>) {
    use winreg::{
        RegKey,
        enums::{
            HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        },
    };
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let Ok(root) = RegKey::predef(hive)
                .open_subkey_with_flags(r"Software\Classes\Applications", KEY_READ | view)
            else {
                continue;
            };
            for name in root.enum_keys().flatten() {
                let Ok(entry) = root.open_subkey(&name) else {
                    continue;
                };
                if entry.get_raw_value("NoOpenWith").is_ok()
                    || entry.get_raw_value("NoStartPage").is_ok()
                {
                    continue;
                }
                let Ok(command_key) = entry.open_subkey(r"shell\open\command") else {
                    continue;
                };
                let Ok(command) = command_key.get_value::<String, _>("") else {
                    continue;
                };
                let Some(target) = association_target(&command) else {
                    continue;
                };
                if !crate::portable::gui_executable(&target) {
                    continue;
                }
                let path = target.to_string_lossy().to_string();
                let app_name = friendly_name(&path).unwrap_or_else(|| {
                    target
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                });
                let scope = if hive == HKEY_CURRENT_USER {
                    "用户"
                } else {
                    "系统"
                };
                let app = AppEntry::new(
                    app_name,
                    path,
                    "文件关联应用",
                    format!(r"{scope}\Classes\Applications\{name}"),
                );
                if seen.insert(app.id.clone()) {
                    apps.push(app);
                }
            }
        }
    }
}

#[cfg(windows)]
pub fn scan_installed_apps(apps: &mut Vec<AppEntry>, seen: &mut HashSet<String>) {
    use winreg::{
        RegKey,
        enums::{
            HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        },
    };
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
                KEY_READ | view,
            ) else {
                continue;
            };
            for key in root.enum_keys().flatten() {
                let Ok(entry) = root.open_subkey(&key) else {
                    continue;
                };
                if entry.get_value::<u32, _>("SystemComponent").unwrap_or(0) == 1 {
                    continue;
                }
                let Ok(display_name) = entry.get_value::<String, _>("DisplayName") else {
                    continue;
                };
                let name = installation_name(&display_name);
                if name.is_empty() {
                    continue;
                }
                let location = entry
                    .get_value::<String, _>("InstallLocation")
                    .unwrap_or_default();
                let icon = entry
                    .get_value::<String, _>("DisplayIcon")
                    .unwrap_or_default();
                let Some(target) = installation_target(&name, &location, &icon) else {
                    crate::diagnostics::record(
                        "过滤",
                        &format!("安装记录\\{key} ({name})"),
                        None,
                        "没有可启动的主程序，仅有组件、图标资源或卸载入口",
                    );
                    continue;
                };
                if !crate::portable::gui_executable(&target) {
                    crate::diagnostics::record(
                        "过滤",
                        &target.to_string_lossy(),
                        None,
                        "安装记录指向命令行或辅助程序，未作为图形应用收录",
                    );
                    continue;
                }
                let path = target.to_string_lossy().to_string();
                let scope = if hive == HKEY_CURRENT_USER {
                    "用户"
                } else {
                    "系统"
                };
                let bits = if view == KEY_WOW64_32KEY { "32" } else { "64" };
                let app = AppEntry::new(
                    name,
                    path,
                    &format!("安装记录（{scope} {bits} 位）"),
                    format!("{scope}\\Uninstall\\{key}"),
                );
                if seen.insert(app.id.clone()) {
                    apps.push(app);
                }
            }
        }
    }
}

#[cfg(not(windows))]
pub fn scan_app_paths(_apps: &mut Vec<AppEntry>, _seen: &mut HashSet<String>) {}
#[cfg(not(windows))]
pub fn scan_installed_apps(_apps: &mut Vec<AppEntry>, _seen: &mut HashSet<String>) {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installation_icons_cannot_supply_installers_or_maintenance_tools() {
        assert!(maintenance_description("Microsoft OneDrive (64 bit) Setup"));
        assert!(maintenance_description(
            "Realtek HD Audio Update and remove driver Tool"
        ));
        assert!(!maintenance_description("Microsoft OneDrive"));
        assert!(!maintenance_description("Visual Studio Code"));
        let root = std::env::temp_dir().join(format!("luma-install-icon-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for name in [
                    "OneDriveSetup.exe",
                    "PowerToysUserSetup.exe",
                    "winsdksetup.exe",
                    "Editor.exe",
                ] {
                    let _ = std::fs::remove_file(self.0.join(name));
                }
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        for name in [
            "OneDriveSetup.exe",
            "PowerToysUserSetup.exe",
            "winsdksetup.exe",
            "Editor.exe",
        ] {
            std::fs::write(root.join(name), b"fixture").unwrap();
        }
        for name in [
            "OneDriveSetup.exe",
            "PowerToysUserSetup.exe",
            "winsdksetup.exe",
        ] {
            assert!(!executable(&root.join(name)));
        }
        assert!(executable(&root.join("Editor.exe")));
        assert_eq!(
            installation_target(
                "Editor",
                &root.to_string_lossy(),
                &root.join("PowerToysUserSetup.exe").to_string_lossy()
            ),
            Some(root.join("Editor.exe"))
        );
    }

    #[test]
    #[ignore = "读取本机安装记录；通过 LUMA_EXPECT_APP 指定要验证的应用"]
    fn host_installation_discovery() {
        let expected = std::env::var("LUMA_EXPECT_APP")
            .expect("需要设置 LUMA_EXPECT_APP")
            .to_lowercase();
        assert!(!expected.is_empty());
        let mut apps = Vec::new();
        let mut seen = HashSet::new();
        scan_app_paths(&mut apps, &mut seen);
        scan_installed_apps(&mut apps, &mut seen);
        let found = apps
            .iter()
            .find(|app| app.name.to_lowercase().contains(&expected))
            .expect("未发现指定应用");
        assert!(Path::new(&found.path).is_file());
        assert!(found.hidden_reason.is_none(), "应用不应被噪音过滤");
        println!(
            "已发现: {} | {} | 分类: {}",
            found.name, found.path, found.category
        );
    }

    #[test]
    fn parses_quoted_and_unquoted_display_icon_without_losing_commas() {
        assert_eq!(
            icon_file(r#""C:\Program Files\App\main.exe",-3"#),
            PathBuf::from(r"C:\Program Files\App\main.exe")
        );
        assert_eq!(
            icon_file(r"C:\Apps\main.exe,0"),
            PathBuf::from(r"C:\Apps\main.exe")
        );
        assert_eq!(
            icon_file(r"C:\Apps\a,b.exe"),
            PathBuf::from(r"C:\Apps\a,b.exe")
        );
    }

    #[test]
    fn removes_install_scope_labels_without_changing_app_names() {
        assert_eq!(installation_name("TRAE CN (User)"), "TRAE CN");
        assert_eq!(installation_name("Editor (System)"), "Editor");
        assert_eq!(installation_name("Editor (Preview)"), "Editor (Preview)");
    }

    #[test]
    fn installation_layouts_and_association_commands_resolve_only_the_main_exe() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("luma layouts {}-{nonce}", std::process::id()));
        let directories = [
            root.join("bin"),
            root.join("app-2.9.0"),
            root.join("app-10.0.0"),
        ];
        struct Cleanup {
            root: PathBuf,
            directories: [PathBuf; 3],
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for directory in &self.directories {
                    let _ = std::fs::remove_file(directory.join("Editor.exe"));
                    let _ = std::fs::remove_dir(directory);
                }
                let _ = std::fs::remove_dir(&self.root);
            }
        }
        let _cleanup = Cleanup {
            root: root.clone(),
            directories: directories.clone(),
        };
        for directory in &directories {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join("Editor.exe"), b"fixture").unwrap();
        }
        let exe = directories[0].join("Editor.exe");
        assert_eq!(
            installation_target("Editor", &root.to_string_lossy(), ""),
            Some(exe.clone())
        );
        assert_eq!(
            association_target(&format!("\"{}\" \"%1\"", exe.display())),
            Some(exe.clone())
        );
        assert_eq!(
            association_target(&format!("{} %1", exe.display())),
            Some(exe.clone())
        );
        assert!(association_target("rundll32.exe fake.dll,Entry").is_none());
        std::fs::remove_file(exe).unwrap();
        assert_eq!(
            installation_target("Editor", &root.to_string_lossy(), ""),
            Some(directories[2].join("Editor.exe"))
        );
    }

    #[test]
    fn finds_main_exe_from_user_installation_and_never_uses_dll_or_uninstaller() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("luma-discovery-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for file in ["TRAE CN.exe", "unins000.exe", "icons.dll"] {
                    let _ = std::fs::remove_file(self.0.join(file));
                }
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        for file in ["TRAE CN.exe", "unins000.exe", "icons.dll"] {
            std::fs::write(root.join(file), b"fixture").unwrap();
        }
        let location = root.to_string_lossy();
        let expected = root.join("TRAE CN.exe");
        assert_eq!(
            installation_target(
                "TRAE CN (User)",
                &location,
                &format!("\"{}\",0", expected.display())
            ),
            Some(expected.clone())
        );
        assert_eq!(
            installation_target(
                "TRAE CN",
                &location,
                &root.join("icons.dll").to_string_lossy()
            ),
            Some(expected)
        );
        assert_eq!(
            installation_target("Other", "", &root.join("icons.dll").to_string_lossy()),
            None
        );
        assert_eq!(
            installation_target("Other", "", &root.join("unins000.exe").to_string_lossy()),
            None
        );
    }
}
