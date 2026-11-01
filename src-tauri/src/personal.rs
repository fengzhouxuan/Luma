//! 用户添加的入口和启动覆盖独立保存，不因重新扫描丢失。
use crate::{catalog, config, model::AppEntry};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManualApp {
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
            app.arguments = item.arguments.clone();
            app.working_dir = item.working_dir.clone();
            app.shortcut_path = item.shortcut_path.clone();
            app.hidden_reason = None;
            effective(app, cfg)
        })
        .collect()
}

pub fn effective(mut app: AppEntry, cfg: &config::Config) -> AppEntry {
    if let Some(value) = cfg.launch_overrides.get(&app.id) {
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
    let path = Path::new(&item.path);
    if !path.is_absolute()
        || !path.extension().is_some_and(|ext| {
            ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("appref-ms")
        })
    {
        return Err("请选择 EXE、快捷方式或 ClickOnce 应用入口".into());
    }
    if require_exists && !path.is_file() {
        return Err("应用文件不存在".into());
    }
    validate_working_dir(item.working_dir.as_deref(), require_exists)?;
    if let Some(shortcut) = &item.shortcut_path {
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
    if !path.is_file() {
        return Err("所选应用文件不存在".into());
    }
    let mut entry = if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
    {
        crate::parse_lnk(&path).ok_or("无法解析此快捷方式的应用目标")?
    } else {
        AppEntry::new(
            path.file_stem()
                .ok_or("文件名无效")?
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
pub async fn pick_manual_app(app: tauri::AppHandle) -> Result<Option<ManualApp>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _hold = crate::runtime::hold_visible();
        let window = app.get_webview_window("main").ok_or("主窗口不可用")?;
        app.dialog()
            .file()
            .set_parent(&window)
            .add_filter("应用入口", &["exe", "lnk", "appref-ms"])
            .blocking_pick_file()
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
    if !Path::new(path).is_file() {
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
