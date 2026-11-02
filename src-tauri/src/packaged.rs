//! 使用 Windows AppsFolder 发现已注册的商店/MSIX 应用，身份不依赖版本目录。
use crate::model::AppEntry;
use std::collections::HashSet;
use windows::{
    Win32::{
        Foundation::{PROPERTYKEY, SIZE},
        System::Com::{
            CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize,
        },
        System::SystemServices::SFGAO_HIDDEN,
        UI::Shell::{
            AO_NOERRORUI, ApplicationActivationManager, BHID_EnumItems,
            IApplicationActivationManager, IEnumShellItems, IShellItem, IShellItem2,
            IShellItemImageFactory, SHCreateItemFromParsingName, SIGDN_DESKTOPABSOLUTEPARSING,
            SIGDN_NORMALDISPLAY, SIIGBF_ICONONLY, SIIGBF_SCALEUP,
        },
    },
    core::{Interface, PCWSTR, PWSTR, w},
};

const PREFIX: &str = "shell:AppsFolder\\";
const APP_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};
const LINK_TARGET: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0xb9b4b3fc_2b51_4a42_b5d8_324146afcf25),
    pid: 2,
};
const LINK_ARGUMENTS: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0x436f2667_14e2_4feb_b30a_146c53b5b674),
    pid: 100,
};

pub fn is_shell_item(path: &str) -> bool {
    [PREFIX, "::{4234d49b-0245-4df3-b780-3893943456e1}\\"]
        .iter()
        .any(|prefix| {
            path.get(..prefix.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        })
}

pub(crate) struct Apartment(bool);
impl Apartment {
    pub(crate) fn enter() -> Result<Self, String> {
        unsafe {
            let status = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            if status.0 == 0x80010106u32 as i32 {
                return Ok(Self(false)); // Reuse the caller's already initialized MTA.
            }
            status.ok().map_err(|e| e.to_string())?;
        }
        Ok(Self(true))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() }
        }
    }
}

// Shell 分配的字符串必须用 CoTaskMemFree 释放，包括转换失败的情况。
pub(crate) struct ShellString(pub(crate) PWSTR);
impl ShellString {
    pub(crate) fn text(&self) -> Result<String, String> {
        if self.0.is_null() {
            return Err("Shell 未返回字符串".into());
        }
        unsafe { self.0.to_string().map_err(|e| e.to_string()) }
    }
}
impl Drop for ShellString {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.0.cast())) }
    }
}

pub fn app_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix(PREFIX)?;
    let (family, application) = id.split_once('!')?;
    if family.is_empty()
        || application.is_empty()
        || !family.contains('_')
        || id
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | '/' | '"'))
        || application.contains('!')
    {
        return None;
    }
    Some(id)
}

fn entry(name: String, id: String) -> Option<AppEntry> {
    let path = format!("{PREFIX}{id}");
    if name.trim().is_empty() || app_id(&path).is_none() {
        return None;
    }
    Some(AppEntry::new(name, path, "商店 / MSIX 应用", id))
}

fn enumerate() -> Result<Vec<AppEntry>, String> {
    let _apartment = Apartment::enter()?;
    unsafe {
        let folder: IShellItem =
            SHCreateItemFromParsingName(w!("shell:AppsFolder"), None).map_err(|e| e.to_string())?;
        let items: IEnumShellItems = folder
            .BindToHandler(None, &BHID_EnumItems)
            .map_err(|e| e.to_string())?;
        let mut apps = Vec::new();
        loop {
            let mut fetched = 0;
            let mut batch = [None];
            items
                .Next(&mut batch, Some(&mut fetched))
                .map_err(|e| e.to_string())?;
            if fetched == 0 {
                break;
            }
            let Some(item) = batch[0].take() else {
                continue;
            };
            if item
                .GetAttributes(SFGAO_HIDDEN)
                .is_ok_and(|attributes| attributes.0 != 0)
            {
                continue;
            }
            let Ok(item2) = item.cast::<IShellItem2>() else {
                continue;
            };
            let Ok(id) = item2.GetString(&APP_ID).map(ShellString) else {
                continue;
            };
            let Ok(id) = id.text() else {
                continue;
            };
            let Ok(name) = item.GetDisplayName(SIGDN_NORMALDISPLAY).map(ShellString) else {
                continue;
            };
            if let Ok(name) = name.text() {
                if let Some(app) = entry(name.clone(), id.clone()) {
                    apps.push(app);
                } else {
                    let Ok(shell_path) = item
                        .GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)
                        .map(ShellString)
                    else {
                        continue;
                    };
                    let Ok(shell_path) = shell_path.text() else {
                        continue;
                    };
                    // 某些 Win32 项返回裸 AUMID，而不是带 AppsFolder 前缀的解析名。
                    let shell_path = if is_shell_item(&shell_path) {
                        shell_path
                    } else {
                        format!("{PREFIX}{id}")
                    };
                    let property = |key: &PROPERTYKEY| {
                        item2
                            .GetString(key)
                            .ok()
                            .map(ShellString)
                            .and_then(|value| value.text().ok())
                            .unwrap_or_default()
                    };
                    let target = property(&LINK_TARGET);
                    let arguments = property(&LINK_ARGUMENTS);
                    if let Some(app) = desktop_entry(name, id, shell_path, target, arguments) {
                        apps.push(app);
                    }
                }
            }
        }
        Ok(apps)
    }
}

fn desktop_entry(
    name: String,
    id: String,
    shell_path: String,
    target: String,
    arguments: String,
) -> Option<AppEntry> {
    if name.trim().is_empty() || !is_shell_item(&shell_path) {
        return None;
    }
    if !target.is_empty()
        && !std::path::Path::new(&target)
            .extension()
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("appref-ms")
            })
    {
        return None;
    }
    let path = if std::path::Path::new(&target).is_file() {
        target
    } else {
        shell_path.clone()
    };
    let mut app = AppEntry::new(name, path, "Windows 注册应用", id);
    app.arguments = arguments;
    app.shortcut_path = Some(shell_path);
    app.update_identity();
    Some(app)
}

pub fn activate_shell(path: &str) -> Result<(), String> {
    if !is_shell_item(path) {
        return Err("不是受支持的应用 Shell 入口".into());
    }
    let path = path.to_string();
    std::thread::spawn(move || {
        use windows::Win32::UI::{
            Shell::{
                SEE_MASK_FLAG_NO_UI, SEE_MASK_IDLIST, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
                SHGetIDListFromObject, ShellExecuteExW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        };
        let _apartment = Apartment::enter()?;
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
                .map_err(|e| e.to_string())?;
            let pidl = SHGetIDListFromObject(&item).map_err(|e| e.to_string())?;
            struct Pidl(*mut std::ffi::c_void);
            impl Drop for Pidl {
                fn drop(&mut self) {
                    unsafe { CoTaskMemFree(Some(self.0)) }
                }
            }
            let pidl = Pidl(pidl.cast());
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
                lpIDList: pidl.0,
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            ShellExecuteExW(&mut info).map_err(|e| format!("Windows 注册应用启动失败: {e}"))
        }
    })
    .join()
    .map_err(|_| "Shell 激活线程异常".to_string())?
}

pub fn scan(apps: &mut Vec<AppEntry>, seen: &mut HashSet<String>) {
    match std::thread::spawn(enumerate).join() {
        Ok(Ok(found)) => {
            for app in found {
                if app.source == "商店 / MSIX 应用"
                    && let Some(existing) = apps.iter_mut().find(|existing| existing.id == app.id)
                {
                    *existing = app;
                } else if seen.insert(app.id.clone()) {
                    apps.push(app);
                }
            }
        }
        result => {
            let error = match result {
                Ok(Err(error)) => error,
                _ => "Shell 应用枚举线程异常".into(),
            };
            crate::diagnostics::record("扫描", "shell:AppsFolder", None, error);
        }
    }
}

pub fn activate(id: &str, arguments: &str) -> Result<(), String> {
    let id: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
    let arguments: Vec<u16> = arguments.encode_utf16().chain(Some(0)).collect();
    std::thread::spawn(move || {
        let _apartment = Apartment::enter()?;
        unsafe {
            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)
                    .map_err(|e| e.to_string())?;
            manager
                .ActivateApplication(
                    PCWSTR(id.as_ptr()),
                    PCWSTR(arguments.as_ptr()),
                    AO_NOERRORUI,
                )
                .map(|_| ())
                .map_err(|e| format!("Windows 应用激活失败: {e}"))
        }
    })
    .join()
    .map_err(|_| "应用激活线程异常".to_string())?
}

// 由已有的图标 STA 线程调用；不访问受保护的 WindowsApps 安装目录。
pub fn icon(path: &str) -> Result<String, String> {
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).map_err(|e| e.to_string())?;
        let bitmap = factory
            .GetImage(SIZE { cx: 96, cy: 96 }, SIIGBF_ICONONLY | SIIGBF_SCALEUP)
            .map_err(|e| e.to_string())?;
        crate::icon_resource::shell_bitmap(bitmap.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_registration_keeps_arguments_and_deduplicates_by_target() {
        let target = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let shell_path = "::{4234D49B-0245-4DF3-B780-3893943456E1}\\Fixture.App";
        let app = desktop_entry(
            "Editor".into(),
            "Fixture.App".into(),
            shell_path.into(),
            target.clone(),
            "--profile Work".into(),
        )
        .unwrap();
        assert_eq!(app.id, crate::model::identity(&target, "--profile Work"));
        assert_eq!(app.shortcut_path.as_deref(), Some(shell_path));
        assert!(
            desktop_entry(
                "Document".into(),
                "Fixture.Doc".into(),
                shell_path.into(),
                "C:\\readme.html".into(),
                String::new()
            )
            .is_none()
        );
    }
    #[test]
    fn missing_shell_property_is_safe_to_skip() {
        assert!(ShellString(PWSTR::null()).text().is_err());
    }
    #[test]
    fn packaged_identity_survives_updates_and_rejects_desktop_ids() {
        let first = entry("ChatGPT".into(), "OpenAI.Example_12345!App".into()).unwrap();
        let renamed = entry("ChatGPT 新版".into(), "OpenAI.Example_12345!App".into()).unwrap();
        assert_eq!(first.id, renamed.id);
        assert!(entry("Helper".into(), "Desktop.App".into()).is_none());
        assert!(entry("Helper".into(), "Family_123!App\\other".into()).is_none());
        assert!(entry("Helper".into(), "Family_123!App!extra".into()).is_none());
        assert!(
            crate::launch::launch_entry(&first, true)
                .unwrap_err()
                .contains("不支持以管理员")
        );
    }
    #[test]
    #[ignore = "读取本机商店应用；通过 LUMA_EXPECT_APP 指定要验证的应用"]
    fn host_packaged_discovery_and_icon() {
        let expected = std::env::var("LUMA_EXPECT_APP")
            .expect("需要设置 LUMA_EXPECT_APP")
            .to_lowercase();
        assert!(!expected.is_empty());
        let apps = enumerate().unwrap();
        let found = apps
            .iter()
            .find(|app| app.name.to_lowercase().contains(&expected))
            .expect("未发现指定商店应用");
        assert!(found.hidden_reason.is_none());
        let _apartment = Apartment::enter().unwrap();
        let data = icon(&found.path).unwrap();
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD
            .decode(data.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let image = image::load_from_memory(&png).unwrap().to_rgba8();
        assert!(
            image.width() > 0 && image.width() <= 96 && image.height() > 0 && image.height() <= 96
        );
        assert!(image.pixels().any(|pixel| pixel[3] != 0));
        println!("已发现: {} | {} | Shell 图标正常", found.name, found.path);
    }

    #[test]
    #[ignore = "激活指定本机商店应用；通过 LUMA_ACTIVATE_APP 指定完整 AUMID"]
    fn host_packaged_activation() {
        let id = std::env::var("LUMA_ACTIVATE_APP").expect("需要设置 LUMA_ACTIVATE_APP");
        let app = enumerate()
            .unwrap()
            .into_iter()
            .find(|app| app.source_path == id)
            .expect("指定应用未注册");
        crate::launch::launch_entry(&app, false).unwrap();
        activate_shell(&format!("{PREFIX}{id}")).unwrap();
        println!("已激活: {} | {}", app.name, id);
    }

    #[test]
    #[ignore = "读取本机 AppsFolder 桌面入口；通过 LUMA_EXPECT_APP 指定应用"]
    fn host_desktop_registration_and_shell_icon() {
        let expected = std::env::var("LUMA_EXPECT_APP")
            .expect("需要指定应用")
            .to_lowercase();
        let apps = enumerate().unwrap();
        let app = apps
            .iter()
            .find(|app| {
                app.name.to_lowercase().contains(&expected) && app.source == "Windows 注册应用"
            })
            .expect("未发现注册桌面应用");
        assert!(std::path::Path::new(&app.path).is_file());
        let _apartment = Apartment::enter().unwrap();
        assert!(
            icon(app.shortcut_path.as_deref().unwrap())
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        println!(
            "桌面注册应用: {} | {} | {}",
            app.name,
            app.path,
            app.shortcut_path.as_deref().unwrap()
        );
    }
}
