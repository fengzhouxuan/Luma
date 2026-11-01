//! 使用当前用户 Run 项；旧版计划任务只作为迁移来源。
use serde::Serialize;
use std::{path::Path, process::Command};
use winreg::{RegKey, enums::*};
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[derive(Serialize)]
pub struct Status {
    pub enabled: bool,
    pub legacy: bool,
    pub command: Option<String>,
}

fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

fn legacy_enabled() -> bool {
    hidden_command("schtasks.exe")
        .args(["/Query", "/TN", "LaunchpadElevated", "/XML"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| {
            let text =
                if output.stdout.starts_with(&[0xff, 0xfe]) || output.stdout.get(1) == Some(&0) {
                    let offset = if output.stdout.starts_with(&[0xff, 0xfe]) {
                        2
                    } else {
                        0
                    };
                    String::from_utf16_lossy(
                        &output.stdout[offset..]
                            .chunks_exact(2)
                            .map(|b| u16::from_le_bytes([b[0], b[1]]))
                            .collect::<Vec<_>>(),
                    )
                } else {
                    String::from_utf8_lossy(&output.stdout).to_string()
                };
            task_settings_enabled(&text)
        })
}

fn task_settings_enabled(xml: &str) -> bool {
    // Task Scheduler 会省略默认值 true；只检查 Settings，不能误读触发器自己的 Enabled。
    let compact: String = xml.chars().filter(|value| !value.is_whitespace()).collect();
    let settings = compact
        .split_once("<Settings>")
        .and_then(|(_, rest)| rest.split_once("</Settings>"))
        .map_or("", |(settings, _)| settings);
    !settings.contains("<Enabled>false</Enabled>") && !settings.contains("<Enabled>0</Enabled>")
}

fn command_for(exe: &Path) -> Result<String, String> {
    let path = exe.to_string_lossy();
    if !exe.is_absolute() || path.contains(['"', '\0']) {
        return Err("自启程序路径无效".into());
    }
    let command = format!("\"{path}\"");
    if command.encode_utf16().count() > 260 {
        return Err("安装路径过长，Windows 自启命令不能超过 260 字符".into());
    }
    Ok(command)
}

fn user_command() -> Option<String> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN)
        .ok()
        .and_then(|key| key.get_value("Luma").ok())
}

pub fn status() -> Status {
    let command = user_command();
    let legacy = legacy_enabled();
    Status {
        enabled: command.is_some() || legacy,
        legacy,
        command,
    }
}

pub fn sync() -> bool {
    // 已启用的自启项跟随当前安装路径；首次安装不擅自开启自启。
    if user_command().is_some() {
        let registered: Option<String> = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Luma")
            .ok()
            .and_then(|key| key.get_value("InstallLocation").ok());
        let exe = std::env::current_exe().ok();
        if registered.zip(exe).is_some_and(|(directory, exe)| {
            crate::model::identity(
                &Path::new(directory.trim_matches('"'))
                    .join("luma.exe")
                    .to_string_lossy(),
                "",
            ) == crate::model::identity(&exe.to_string_lossy(), "")
        }) {
            if let Err(error) = write(true) {
                crate::diagnostics::record("自启", "当前用户", None, error);
            }
        }
        true
    } else {
        legacy_enabled()
    }
}

fn write(enabled: bool) -> Result<(), String> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(RUN)
        .map_err(|e| e.to_string())?;
    write_value(
        &key,
        enabled,
        &std::env::current_exe().map_err(|e| e.to_string())?,
    )
}

fn write_value(key: &RegKey, enabled: bool, exe: &Path) -> Result<(), String> {
    if enabled {
        key.set_value("Luma", &command_for(exe)?)
            .map_err(|e| e.to_string())
    } else {
        match key.delete_value("Luma") {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[tauri::command]
pub async fn get_autostart_status() -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(status)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_autostart(enabled: bool) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if legacy_enabled() {
            let output = hidden_command("schtasks.exe").args(["/Change", "/TN", "LaunchpadElevated", "/Disable"]).output().map_err(|e| e.to_string())?;
            if !output.status.success() { return Err("Windows 拒绝停用旧版 LaunchpadElevated 计划任务。请先由管理员停用该旧任务，再重试；当前自启项未更改。".into()); }
        }
        write(enabled)?;
        crate::config::update(|cfg| cfg.autostart = enabled)?;
        Ok(status())
    }).await.map_err(|e| e.to_string())?
}

/// 仅由用户点击迁移入口后经 Windows UAC 启动，不创建窗口或常驻进程。
pub fn migration_helper() -> bool {
    if !std::env::args_os().any(|arg| arg == "--disable-legacy-autostart") {
        return false;
    }
    let success = hidden_command("schtasks.exe")
        .args(["/Change", "/TN", "LaunchpadElevated", "/Disable"])
        .status()
        .is_ok_and(|status| status.success());
    std::process::exit(if success { 0 } else { 1 });
}

#[tauri::command]
pub async fn migrate_legacy_autostart() -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let _hold = crate::runtime::hold_visible();
        if legacy_enabled() {
            use windows_sys::Win32::{
                Foundation::{CloseHandle, GetLastError},
                System::Threading::{GetExitCodeProcess, WaitForSingleObject},
                UI::{
                    Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
                    WindowsAndMessaging::SW_HIDE,
                },
            };
            let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let operation = wide("runas");
            let file = wide(
                &std::env::current_exe()
                    .map_err(|e| e.to_string())?
                    .to_string_lossy(),
            );
            let args = wide("--disable-legacy-autostart");
            let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
            info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
            info.fMask = SEE_MASK_NOCLOSEPROCESS;
            info.lpVerb = operation.as_ptr();
            info.lpFile = file.as_ptr();
            info.lpParameters = args.as_ptr();
            info.nShow = SW_HIDE;
            if unsafe { ShellExecuteExW(&mut info) } == 0 {
                return Err(format!(
                    "管理员迁移未完成或已取消（Windows {}），旧自启保持原状。",
                    unsafe { GetLastError() }
                ));
            }
            if info.hProcess.is_null() {
                return Err("未获得迁移进程状态，请重新检查自启状态".into());
            }
            let wait = unsafe { WaitForSingleObject(info.hProcess, 300_000) };
            let mut code = 1u32;
            let read = unsafe { GetExitCodeProcess(info.hProcess, &mut code) };
            unsafe {
                CloseHandle(info.hProcess);
            }
            if wait != 0 || read == 0 || code != 0 || legacy_enabled() {
                return Err("旧任务迁移未完成，请重试或请管理员停用 LaunchpadElevated。".into());
            }
        }
        // 新的登录项由原来的普通用户进程写入，避免管理员账号不同而写错 HKCU。
        write(true)?;
        crate::config::update(|cfg| cfg.autostart = true)?;
        Ok(status())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quotes_user_install_path_and_rejects_invalid_run_commands() {
        assert_eq!(
            command_for(Path::new(r"C:\Program Files\Luma\luma.exe")).unwrap(),
            r#""C:\Program Files\Luma\luma.exe""#
        );
        assert!(command_for(Path::new("C:\\bad\"path\\luma.exe")).is_err());
        assert!(command_for(Path::new("relative.exe")).is_err());
        assert!(command_for(Path::new(&format!(r"C:\{}\luma.exe", "x".repeat(260)))).is_err());
    }
    #[test]
    fn omitted_enabled_setting_defaults_to_true_and_trigger_flags_are_separate() {
        assert!(task_settings_enabled(
            "<Task><Settings></Settings><Triggers><Enabled>false</Enabled></Triggers></Task>"
        ));
        assert!(task_settings_enabled(
            "<Task><Settings><Enabled>true</Enabled></Settings></Task>"
        ));
        assert!(!task_settings_enabled(
            "<Task><Settings> <Enabled> false </Enabled> </Settings></Task>"
        ));
        assert!(!task_settings_enabled(
            "<Task><Settings><Enabled>0</Enabled></Settings></Task>"
        ));
    }
    #[test]
    fn run_value_can_be_enabled_updated_and_disabled_in_an_isolated_registry_key() {
        let parent = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Software", KEY_ALL_ACCESS)
            .unwrap();
        let name = format!(
            "Luma-autostart-test-{}-{}",
            std::process::id(),
            crate::diagnostics::now()
        );
        let (key, _) = parent.create_subkey(&name).unwrap();
        write_value(&key, true, Path::new(r"C:\Luma\luma.exe")).unwrap();
        assert_eq!(
            key.get_value::<String, _>("Luma").unwrap(),
            r#""C:\Luma\luma.exe""#
        );
        write_value(&key, true, Path::new(r"D:\New Install\luma.exe")).unwrap();
        assert_eq!(
            key.get_value::<String, _>("Luma").unwrap(),
            r#""D:\New Install\luma.exe""#
        );
        write_value(&key, false, Path::new(r"D:\New Install\luma.exe")).unwrap();
        write_value(&key, false, Path::new(r"D:\New Install\luma.exe")).unwrap();
        assert!(key.get_value::<String, _>("Luma").is_err());
        drop(key);
        parent.delete_subkey(&name).unwrap();
    }
}
