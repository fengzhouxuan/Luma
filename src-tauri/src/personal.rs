//! 用户添加的入口和启动覆盖独立保存，不因重新扫描丢失。
use crate::{
    catalog, config,
    model::{AppEntry, EntryKind},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManualApp {
    #[serde(default)]
    pub kind: EntryKind,
    pub id: String,
    pub name: String,
    pub path: String,
    pub arguments: String,
    pub working_dir: Option<String>,
    #[serde(default)]
    pub shortcut_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchOverride {
    pub arguments: String,
    pub working_dir: Option<String>,
}

pub fn entries(cfg: &config::Config) -> Vec<AppEntry> {
    cfg.manual_apps
        .iter()
        .map(|item| {
            let mut app = AppEntry::new(
                item.name.clone(),
                item.path.clone(),
                "手动添加",
                item.path.clone(),
            );
            app.id = item.id.clone();
            app.kind = item.kind;
            match item.kind {
                EntryKind::Web => app.category = "浏览器".into(),
                EntryKind::Script => app.category = "开发".into(),
                EntryKind::Folder => app.category = "其他".into(),
                EntryKind::App => (),
            }
            app.arguments = item.arguments.clone();
            app.working_dir = item.working_dir.clone();
            app.shortcut_path = item.shortcut_path.clone();
            app.hidden_reason = None;
            effective(app, cfg)
        })
        .collect()
}

pub fn effective(mut app: AppEntry, cfg: &config::Config) -> AppEntry {
    if let Some(value) = cfg
        .launch_overrides
        .get(&app.id)
        .filter(|_| matches!(app.kind, EntryKind::App | EntryKind::Script))
    {
        app.arguments = value.arguments.clone();
        app.working_dir = value.working_dir.clone();
        // 保留快捷方式供图标和详情使用；启动层遇到覆盖时跳过旧快捷方式。
    }
    app
}

fn text(value: &str, max: usize) -> Result<(), String> {
    if value.len() > max || value.contains('\0') {
        Err("内容过长或包含无效字符".into())
    } else {
        Ok(())
    }
}

pub fn validate(item: &ManualApp, require_exists: bool) -> Result<(), String> {
    if !item.id.starts_with("manual-") || item.id.len() > 120 || item.name.trim().is_empty() {
        return Err("手动应用名称或标识无效".into());
    }
    text(&item.name, 256)?;
    text(&item.path, 32768)?;
    text(&item.arguments, 32768)?;
    validate_target(item.kind, &item.path, require_exists)?;
    if matches!(item.kind, EntryKind::Web | EntryKind::Folder)
        && (!item.arguments.is_empty()
            || item.working_dir.is_some()
            || item.shortcut_path.is_some())
    {
        return Err("网页和文件夹入口不支持启动参数或快捷方式".into());
    }
    validate_working_dir(item.working_dir.as_deref(), require_exists)?;
    if let Some(shortcut) = &item.shortcut_path {
        if item.kind != EntryKind::App {
            return Err("仅应用入口支持快捷方式".into());
        }
        text(shortcut, 32768)?;
        if !Path::new(shortcut).is_absolute()
            || !Path::new(shortcut)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
        {
            return Err("快捷方式路径无效".into());
        }
    }
    Ok(())
}

pub fn validate_target(kind: EntryKind, target: &str, require_exists: bool) -> Result<(), String> {
    text(target, 32768)?;
    if kind == EntryKind::Web {
        let url = reqwest::Url::parse(target).map_err(|_| "请输入完整的 http 或 https 网页地址")?;
        return if matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && !target.chars().any(char::is_control)
        {
            Ok(())
        } else {
            Err("网页地址仅支持 http/https，且不能包含登录凭据".into())
        };
    }
    let path = Path::new(target);
    if !path.is_absolute() || target.contains(['"', '\r', '\n']) {
        return Err("请选择有效的绝对路径".into());
    }
    let extensions: &[&str] = match kind {
        EntryKind::App => &["exe", "appref-ms"],
        EntryKind::Script => &["bat", "cmd", "ps1"],
        _ => &[],
    };
    if !extensions.is_empty()
        && !path.extension().is_some_and(|ext| {
            extensions
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
    {
        return Err(if kind == EntryKind::Script {
            "请选择 BAT、CMD 或 PowerShell 脚本"
        } else {
            "请选择 EXE、快捷方式或 ClickOnce 应用入口"
        }
        .into());
    }
    if kind == EntryKind::Script
        && target.contains('%')
        && !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ps1"))
    {
        return Err("BAT/CMD 脚本路径不能包含 %，请更换路径后重试".into());
    }
    if require_exists
        && !(if kind == EntryKind::Folder {
            path.is_dir()
        } else {
            path.is_file()
        })
    {
        return Err("入口目标不存在或类型不匹配".into());
    }
    Ok(())
}

pub fn is_missing(item: &ManualApp) -> bool {
    match item.kind {
        EntryKind::Web => false,
        EntryKind::Folder => !Path::new(&item.path).is_dir(),
        _ => !Path::new(&item.path).is_file(),
    }
}

pub fn validate_working_dir(directory: Option<&str>, require_exists: bool) -> Result<(), String> {
    if let Some(directory) = directory {
        text(directory, 32768)?;
        if !Path::new(directory).is_absolute() || (require_exists && !Path::new(directory).is_dir())
        {
            return Err("工作目录必须是有效的绝对目录".into());
        }
    }
    Ok(())
}

pub fn prepare(path: &str) -> Result<ManualApp, String> {
    let path = PathBuf::from(path.trim().trim_matches('"'));
    if !path.exists() {
        return Err("所选应用文件不存在".into());
    }
    let mut entry = if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
    {
        crate::parse_lnk(&path).ok_or("无法解析此快捷方式的应用目标")?
    } else {
        AppEntry::new(
            (if path.is_dir() {
                path.file_name()
            } else {
                path.file_stem()
            })
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .to_string(),
            path.to_string_lossy().to_string(),
            "手动添加",
            String::new(),
        )
    };
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    entry.id = format!(
        "manual-{:x}-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let item = ManualApp {
        kind: if path.is_dir() {
            EntryKind::Folder
        } else if path.extension().is_some_and(|ext| {
            ["bat", "cmd", "ps1"]
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        }) {
            EntryKind::Script
        } else {
            EntryKind::App
        },
        id: entry.id,
        name: entry.name,
        path: entry.path,
        arguments: entry.arguments,
        working_dir: entry.working_dir,
        shortcut_path: entry.shortcut_path,
    };
    validate(&item, true)?;
    Ok(item)
}

#[tauri::command]
pub async fn pick_manual_app(
    app: tauri::AppHandle,
    kind: Option<EntryKind>,
) -> Result<Option<ManualApp>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _hold = crate::runtime::hold_visible();
        let window = app.get_webview_window("main").ok_or("主窗口不可用")?;
        let picker = app.dialog().file().set_parent(&window);
        let selected = match kind.unwrap_or_default() {
            EntryKind::Folder => picker.blocking_pick_folder(),
            EntryKind::Script => picker
                .add_filter("脚本", &["bat", "cmd", "ps1"])
                .blocking_pick_file(),
            EntryKind::App => picker
                .add_filter("应用入口", &["exe", "lnk", "appref-ms"])
                .blocking_pick_file(),
            EntryKind::Web => return Err("请直接输入网页地址".into()),
        };
        selected
            .map(|file| {
                file.into_path()
                    .map_err(|e| e.to_string())
                    .and_then(|path| prepare(&path.to_string_lossy()))
            })
            .transpose()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn prepare_manual_app(path: String) -> Result<ManualApp, String> {
    tauri::async_runtime::spawn_blocking(move || prepare(&path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn save_manual_app(item: ManualApp, aliases: Vec<String>) -> Result<config::Config, String> {
    validate(&item, true)?;
    let aliases = validate_aliases(aliases)?;
    let custom_launch = item
        .shortcut_path
        .as_ref()
        .and_then(|path| crate::parse_lnk(&PathBuf::from(path)))
        .is_some_and(|original| {
            original.arguments != item.arguments || original.working_dir != item.working_dir
        });
    config::update(|cfg| {
        if let Some(current) = cfg
            .manual_apps
            .iter_mut()
            .find(|current| current.id == item.id)
        {
            *current = item.clone();
        } else {
            cfg.manual_apps.push(item.clone());
        }
        cfg.aliases.insert(item.id.clone(), aliases);
        if custom_launch {
            cfg.launch_overrides.insert(
                item.id.clone(),
                LaunchOverride {
                    arguments: item.arguments.clone(),
                    working_dir: item.working_dir.clone(),
                },
            );
        } else {
            cfg.launch_overrides.remove(&item.id);
        }
    })?;
    catalog::preferences_changed();
    Ok(config::get())
}

pub fn validate_aliases(aliases: Vec<String>) -> Result<Vec<String>, String> {
    if aliases.len() > 32 {
        return Err("每个应用最多设置 32 个别名".into());
    }
    let mut result = Vec::new();
    for alias in aliases {
        text(&alias, 256)?;
        let alias = alias.trim().to_string();
        if !alias.is_empty()
            && !result
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&alias))
        {
            result.push(alias);
        }
    }
    Ok(result)
}

#[tauri::command]
pub fn save_app_options(
    app_id: String,
    aliases: Vec<String>,
    launch: Option<LaunchOverride>,
) -> Result<config::Config, String> {
    let app = catalog::find(&app_id).ok_or("应用记录已失效，请刷新")?;
    let aliases = validate_aliases(aliases)?;
    if let Some(value) = &launch {
        if app.path.starts_with("shell:") || app.path.starts_with("::{") || app.path.contains("://")
        {
            return Err("此 Windows 管理入口只支持设置别名".into());
        }
        text(&value.arguments, 32768)?;
        validate_working_dir(value.working_dir.as_deref(), true)?;
    }
    config::update(|cfg| {
        cfg.aliases.insert(app_id.clone(), aliases);
        if let Some(value) = launch {
            cfg.launch_overrides.insert(app_id.clone(), value);
        } else {
            cfg.launch_overrides.remove(&app_id);
        }
    })?;
    catalog::preferences_changed();
    Ok(config::get())
}

#[tauri::command]
pub fn remove_manual_app(app_id: String) -> Result<config::Config, String> {
    config::update(|cfg| {
        cfg.manual_apps.retain(|item| item.id != app_id);
        cfg.aliases.remove(&app_id);
        cfg.launch_overrides.remove(&app_id);
        cfg.usage.remove(&app_id);
        cfg.app_categories.remove(&app_id);
        cfg.pinned_apps.retain(|id| id != &app_id);
        cfg.hidden_apps.retain(|id| id != &app_id);
    })?;
    catalog::preferences_changed();
    Ok(config::get())
}

#[tauri::command]
pub fn copy_app_path(app: tauri::AppHandle, app_id: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let entry = catalog::find(&app_id).ok_or("应用记录已失效，请刷新")?;
    app.clipboard()
        .write_text(entry.path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reveal_app(app: tauri::AppHandle, app_id: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let entry = catalog::find(&app_id).ok_or("应用记录已失效，请刷新")?;
    let path = entry
        .shortcut_path
        .as_deref()
        .filter(|path| Path::new(path).is_file())
        .unwrap_or(&entry.path);
    if !Path::new(path).exists() {
        return Err("该入口由 Windows 管理，没有可打开的本地文件".into());
    }
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_targets_reject_protocol_commands_and_preserve_legacy_apps() {
        let old = r#"{"id":"manual-old","name":"旧入口","path":"C:\\Apps\\old.exe","arguments":"","working_dir":null}"#;
        let mut item: ManualApp = serde_json::from_str(old).unwrap();
        assert_eq!(item.kind, EntryKind::App);
        assert!(validate(&item, false).is_ok());
        item.kind = EntryKind::Web;
        item.path = "https://example.com/中文?q=One%20Two#section".into();
        assert!(validate(&item, true).is_ok());
        assert!(!is_missing(&item));
        for target in [
            "file:///C:/Windows",
            "javascript:alert(1)",
            "https://",
            "https://user:pass@example.com",
            "https://example.com\n",
        ] {
            item.path = target.into();
            assert!(validate(&item, false).is_err(), "{target}");
        }
        item.path = "https://example.com".into();
        item.arguments = "--command".into();
        assert!(validate(&item, false).is_err());
        assert!(validate_target(EntryKind::Script, r"C:\测试 & 工作\launch.cmd", false).is_ok());
        assert!(validate_target(EntryKind::Script, r"C:\%TEMP%\launch.cmd", false).is_err());
        assert!(validate_target(EntryKind::Script, r"C:\test.exe", false).is_err());
    }

    #[test]
    fn prepare_distinguishes_folders_and_scripts_and_checks_target_type() {
        let dir = std::env::temp_dir().join(format!(
            "luma-entry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let script = dir.join("测试 script.ps1");
        std::fs::write(&script, "# test fixture").unwrap();
        let folder = prepare(dir.to_str().unwrap()).unwrap();
        let script_entry = prepare(script.to_str().unwrap()).unwrap();
        assert_eq!(folder.kind, EntryKind::Folder);
        assert_eq!(script_entry.kind, EntryKind::Script);
        assert!(validate_target(EntryKind::Folder, script.to_str().unwrap(), true).is_err());
        assert!(!is_missing(&folder));
        std::fs::remove_file(script).unwrap();
        std::fs::remove_dir(dir).unwrap();
        assert!(is_missing(&folder));
        assert!(is_missing(&script_entry));
    }
    #[test]
    fn aliases_are_bounded_trimmed_and_deduplicated() {
        assert_eq!(
            validate_aliases(vec![
                " 编辑器 ".into(),
                "code".into(),
                "CODE".into(),
                "".into()
            ])
            .unwrap(),
            vec!["编辑器", "code"]
        );
        assert!(validate_aliases(vec!["x".repeat(257)]).is_err());
    }
    #[test]
    fn overrides_keep_identity_and_shortcut_icon_resource() {
        let mut app = AppEntry::new(
            "Editor".into(),
            r"C:\Editor.exe".into(),
            "开始菜单",
            String::new(),
        );
        app.shortcut_path = Some(r"C:\Editor.lnk".into());
        let id = app.id.clone();
        let mut cfg = config::Config::default();
        cfg.launch_overrides.insert(
            id.clone(),
            LaunchOverride {
                arguments: "--profile work".into(),
                working_dir: Some(r"C:\Work".into()),
            },
        );
        let result = effective(app, &cfg);
        assert_eq!(result.id, id);
        assert_eq!(result.arguments, "--profile work");
        assert_eq!(result.shortcut_path.as_deref(), Some(r"C:\Editor.lnk"));
    }
    #[test]
    fn imported_missing_apps_remain_editable_but_cannot_be_saved_as_valid_existing_files() {
        let item = ManualApp {
            kind: EntryKind::App,
            id: "manual-example".into(),
            name: "Example".into(),
            path: r"Z:\missing\example.exe".into(),
            arguments: String::new(),
            working_dir: None,
            shortcut_path: None,
        };
        assert!(validate(&item, false).is_ok());
        assert!(validate(&item, true).is_err());
    }
}
