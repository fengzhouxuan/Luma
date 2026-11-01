use crate::{catalog, config, diagnostics, model::AppEntry};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
pub struct LaunchResult {
    pub usage: Option<config::Usage>,
}

#[tauri::command]
pub async fn launch_app(app_id: String, admin: bool) -> Result<LaunchResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let app = catalog::find(&app_id).ok_or("应用记录已失效，请刷新后重试")?;
        match launch_entry(&app, admin) {
            Ok(()) => {
                diagnostics::record("启动", &app.path, Some(&app.id), "启动成功");
                Ok(LaunchResult {
                    usage: config::record_launch(&app.id).ok(),
                })
            }
            Err(error) => {
                diagnostics::record("启动", &app.path, Some(&app.id), &error);
                Err(error)
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

pub fn launch_entry(app: &AppEntry, admin: bool) -> Result<(), String> {
    #[cfg(windows)]
    if let Some(id) = crate::packaged::app_id(&app.path) {
        if admin {
            return Err("商店 / MSIX 应用由 Windows 管理启动，不支持以管理员身份启动".into());
        }
        return crate::packaged::activate(id, &app.arguments);
    }
    #[cfg(windows)]
    if crate::packaged::is_shell_item(&app.path) {
        if admin {
            return Err("该注册应用没有直接程序路径，不支持以管理员身份启动".into());
        }
        return crate::packaged::activate_shell(&app.path);
    }
    if admin
        && (app.path.contains("://")
            || Path::new(&app.path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("appref-ms")))
    {
        return Err("协议或 ClickOnce 入口不支持以管理员身份启动".into());
    }
    // 普通启动交给 Shell 打开原快捷方式，保留其全部启动语义。
    if !admin {
        #[cfg(windows)]
        if let Some(shortcut) = app
            .shortcut_path
            .as_deref()
            .filter(|path| crate::packaged::is_shell_item(path))
        {
            return crate::packaged::activate_shell(shortcut);
        }
        if let Some(shortcut) = app
            .shortcut_path
            .as_deref()
            .filter(|p| Path::new(p).is_file())
        {
            if shell_execute(shortcut, "", None, false).is_ok() {
                return Ok(());
            }
        }
    }
    // 管理员启动或快捷方式损坏时，仍保留解析出的原始参数和工作目录。
    let directory = app
        .working_dir
        .as_deref()
        .filter(|path| Path::new(path).is_dir())
        .map(str::to_string)
        .or_else(|| {
            Path::new(&app.path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
        });
    shell_execute(&app.path, &app.arguments, directory.as_deref(), admin)
}

#[cfg(windows)]
fn shell_execute(
    path: &str,
    arguments: &str,
    directory: Option<&str>,
    admin: bool,
) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let wide = |text: &str| {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<u16>>()
    };
    let operation = wide(if admin { "runas" } else { "open" });
    let file = wide(path);
    let arguments = wide(arguments);
    let directory = directory.map(wide);
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            arguments.as_ptr(),
            directory
                .as_ref()
                .map_or(std::ptr::null(), |path| path.as_ptr()),
            SW_SHOWNORMAL,
        )
    };
    if result as isize > 32 {
        Ok(())
    } else {
        Err(format!("启动失败，Windows 错误码: {}", result as isize))
    }
}

#[cfg(not(windows))]
fn shell_execute(
    _path: &str,
    _arguments: &str,
    _directory: Option<&str>,
    _admin: bool,
) -> Result<(), String> {
    Err("当前平台不支持".into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn shortcut_and_direct_fallback_preserve_unicode_arguments_and_working_directory() {
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                for file in [
                    "helper.rs",
                    "helper.exe",
                    "helper.pdb",
                    "profile.lnk",
                    "result.txt",
                ] {
                    for _ in 0..20 {
                        match std::fs::remove_file(self.0.join(file)) {
                            Ok(()) => break,
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                            Err(_) => std::thread::sleep(Duration::from_millis(20)),
                        }
                    }
                }
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fixture = Fixture(
            std::env::temp_dir().join(format!("luma launch 测试 {}-{nonce}", std::process::id())),
        );
        std::fs::create_dir(&fixture.0).unwrap();
        let source = fixture.0.join("helper.rs");
        let exe = fixture.0.join("helper.exe");
        let result = fixture.0.join("result.txt");
        std::fs::write(&source, r#"#![windows_subsystem = "windows"]
fn main() {
    let mut args = std::env::args().skip(1);
    let output = args.next().unwrap();
    let text = format!("{}\n{}", std::env::current_dir().unwrap().display(), args.collect::<Vec<_>>().join("\n"));
    std::fs::write(output, text).unwrap();
}"#).unwrap();
        let compile = std::process::Command::new("rustc")
            .args(["--edition=2024", "--crate-name", "luma_launch_fixture"])
            .arg(&source)
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let create = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", r#"
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut((Join-Path $env:LUMA_TEST_ROOT 'profile.lnk'))
$shortcut.TargetPath = Join-Path $env:LUMA_TEST_ROOT 'helper.exe'
$shortcut.Arguments = '"' + (Join-Path $env:LUMA_TEST_ROOT 'result.txt') + '" "hello world" "--profile=中文"'
$shortcut.WorkingDirectory = $env:LUMA_TEST_ROOT
$shortcut.Save()
"#]).env("LUMA_TEST_ROOT", &fixture.0).output().unwrap();
        assert!(
            create.status.success(),
            "{}",
            String::from_utf8_lossy(&create.stderr)
        );
        use windows_sys::Win32::System::Com::{
            COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
        };
        let hr = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        assert!(hr >= 0);
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _com = Com;
        let shortcut = fixture.0.join("profile.lnk");
        let app = crate::parse_lnk(&shortcut).unwrap();
        assert!(app.arguments.contains("hello world") && app.arguments.contains("--profile=中文"));
        let assert_result = || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !result.is_file() {
                assert!(Instant::now() < deadline, "启动的测试程序没有输出");
                std::thread::sleep(Duration::from_millis(30));
            }
            let text = std::fs::read_to_string(&result).unwrap();
            let lines: Vec<_> = text.lines().collect();
            assert_eq!(
                std::fs::canonicalize(lines[0]).unwrap(),
                std::fs::canonicalize(&fixture.0).unwrap()
            );
            assert_eq!(&lines[1..], &["hello world", "--profile=中文"]);
        };
        launch_entry(&app, false).unwrap();
        assert_result();
        std::fs::remove_file(&result).unwrap();
        std::fs::remove_file(&shortcut).unwrap();
        launch_entry(&app, false).unwrap();
        assert_result();
    }
}
