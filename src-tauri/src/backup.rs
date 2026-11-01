use crate::{
    config::{self, Appearance, Config, Usage},
    personal::{LaunchOverride, ManualApp},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::Path,
};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
const LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    appearance: Option<Appearance>,
    show_all: bool,
    extra_dirs: Vec<String>,
    custom_categories: Vec<String>,
    app_categories: HashMap<String, String>,
    pinned_apps: Vec<String>,
    hidden_apps: Vec<String>,
    usage: HashMap<String, Usage>,
    manual_apps: Vec<ManualApp>,
    aliases: HashMap<String, Vec<String>>,
    launch_overrides: HashMap<String, LaunchOverride>,
}
#[derive(Serialize, Deserialize)]
struct Backup {
    format: String,
    version: u32,
    preferences: Preferences,
}
#[derive(Serialize)]
pub struct Preview {
    pub json: String,
    pub manual_apps: usize,
    pub aliases: usize,
    pub categories: usize,
    pub pinned: usize,
    pub hidden: usize,
    pub directories: usize,
    pub missing: Vec<String>,
}

fn from_config(cfg: &Config) -> Backup {
    Backup {
        format: "luma-config".into(),
        version: 1,
        preferences: Preferences {
            appearance: Some(cfg.appearance.clone()),
            show_all: cfg.show_all,
            extra_dirs: cfg.extra_dirs.clone(),
            custom_categories: cfg.custom_categories.clone(),
            app_categories: cfg.app_categories.clone(),
            pinned_apps: cfg.pinned_apps.clone(),
            hidden_apps: cfg.hidden_apps.clone(),
            usage: cfg.usage.clone(),
            manual_apps: cfg.manual_apps.clone(),
            aliases: cfg.aliases.clone(),
            launch_overrides: cfg.launch_overrides.clone(),
        },
    }
}

fn parse(json: &str) -> Result<Backup, String> {
    if json.len() > LIMIT {
        return Err("备份文件不能超过 2MB".into());
    }
    let mut data: Backup = serde_json::from_str(json).map_err(|e| format!("备份格式无效：{e}"))?;
    if data.format != "luma-config" || data.version != 1 {
        return Err("不支持的备份类型或版本".into());
    }
    let p = &mut data.preferences;
    if p.manual_apps.len() > 5000
        || p.aliases.len() > 20000
        || p.launch_overrides.len() > 20000
        || p.usage.len() > 20000
        || p.app_categories.len() > 20000
    {
        return Err("备份条目过多".into());
    }
    let mut ids = HashSet::new();
    for item in &p.manual_apps {
        crate::personal::validate(item, false)?;
        if !ids.insert(&item.id) {
            return Err("备份包含重复的手动应用标识".into());
        }
    }
    for values in p.aliases.values_mut() {
        *values = crate::personal::validate_aliases(std::mem::take(values))?;
    }
    for value in p.launch_overrides.values() {
        if value.arguments.len() > 32768 || value.arguments.contains('\0') {
            return Err("启动参数无效".into());
        }
        crate::personal::validate_working_dir(value.working_dir.as_deref(), false)?;
    }
    for list in [
        &p.extra_dirs,
        &p.custom_categories,
        &p.pinned_apps,
        &p.hidden_apps,
    ] {
        if list.len() > 20000
            || list
                .iter()
                .any(|value| value.len() > 32768 || value.contains('\0'))
        {
            return Err("备份字段无效".into());
        }
    }
    if p.extra_dirs
        .iter()
        .any(|directory| !Path::new(directory).is_absolute())
    {
        return Err("扫描目录必须使用绝对路径".into());
    }
    if let Some(appearance) = &p.appearance {
        if !appearance.opacity.is_finite()
            || !(0.3..=0.95).contains(&appearance.opacity)
            || !appearance.corner_radius.is_finite()
            || !(0.0..=24.0).contains(&appearance.corner_radius)
        {
            return Err("外观配置超出允许范围".into());
        }
    }
    Ok(data)
}

fn apply(cfg: &mut Config, p: Preferences) {
    if let Some(appearance) = p.appearance {
        cfg.appearance = appearance;
    }
    cfg.show_all = p.show_all;
    cfg.extra_dirs = p.extra_dirs;
    cfg.custom_categories = p.custom_categories;
    cfg.app_categories = p.app_categories;
    cfg.pinned_apps = p.pinned_apps;
    cfg.hidden_apps = p.hidden_apps;
    cfg.usage = p.usage;
    cfg.manual_apps = p.manual_apps;
    cfg.aliases = p.aliases;
    cfg.launch_overrides = p.launch_overrides;
    // 热键、自启和窗口位置属于本机环境，不从其他电脑的备份覆盖。
}

#[tauri::command]
pub async fn export_config_backup(app: tauri::AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _hold = crate::runtime::hold_visible();
        let window = app.get_webview_window("main").ok_or("主窗口不可用")?;
        let Some(file) = app
            .dialog()
            .file()
            .set_parent(&window)
            .add_filter("Luma 配置备份", &["json"])
            .set_file_name("Luma-backup.json")
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = file.into_path().map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(&from_config(&config::get()))
            .map_err(|e| e.to_string())?;
        parse(&json)?;
        std::fs::write(&path, json).map_err(|e| e.to_string())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn preview_config_import(app: tauri::AppHandle) -> Result<Option<Preview>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _hold = crate::runtime::hold_visible();
        let window = app.get_webview_window("main").ok_or("主窗口不可用")?;
        let Some(file) = app
            .dialog()
            .file()
            .set_parent(&window)
            .add_filter("Luma 配置备份", &["json"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let path = file.into_path().map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let json = String::from_utf8(bytes).map_err(|_| "备份文件必须使用 UTF-8 编码")?;
        let data = parse(&json)?;
        let p = data.preferences;
        let missing: Vec<_> = p
            .manual_apps
            .iter()
            .filter(|item| !Path::new(&item.path).is_file())
            .map(|item| item.path.clone())
            .chain(
                p.extra_dirs
                    .iter()
                    .filter(|directory| !Path::new(directory).is_dir())
                    .cloned(),
            )
            .collect();
        Ok(Some(Preview {
            json,
            manual_apps: p.manual_apps.len(),
            aliases: p.aliases.values().map(Vec::len).sum(),
            categories: p.custom_categories.len(),
            pinned: p.pinned_apps.len(),
            hidden: p.hidden_apps.len(),
            directories: p.extra_dirs.len(),
            missing,
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn import_config_backup(json: String, app: tauri::AppHandle) -> Result<Config, String> {
    let data = parse(&json)?;
    let dir = std::path::PathBuf::from(std::env::var_os("APPDATA").ok_or("无法定位配置目录")?)
        .join("Luma")
        .join("backups");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = dir.join(format!("before-import-{stamp}.json"));
    let previous =
        serde_json::to_vec_pretty(&from_config(&config::get())).map_err(|e| e.to_string())?;
    std::fs::write(path, previous).map_err(|e| format!("保存导入前备份失败：{e}"))?;
    config::update(|cfg| apply(cfg, data.preferences))?;
    crate::appearance::refresh(&app);
    crate::catalog::invalidate(2);
    crate::catalog::preferences_changed();
    Ok(config::get())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_versions_and_invalid_launch_inputs_before_changes() {
        assert!(parse(r#"{"format":"luma-config","version":99,"preferences":{}}"#).is_err());
        assert!(parse("{}").is_err());
        assert!(parse(&"x".repeat(LIMIT + 1)).is_err());
        let mut value = from_config(&Config::default());
        value.preferences.extra_dirs = vec!["relative/path".into()];
        assert!(parse(&serde_json::to_string(&value).unwrap()).is_err());
    }
    #[test]
    fn round_trip_restores_personalization_without_changing_machine_settings() {
        let mut source = Config::default();
        source.aliases.insert("app".into(), vec!["工作".into()]);
        source.pinned_apps.push("app".into());
        source.custom_categories.push("常用".into());
        let data = parse(&serde_json::to_string(&from_config(&source)).unwrap()).unwrap();
        let mut target = Config::default();
        target.autostart = true;
        target.window_position = Some(crate::position::SavedPosition {
            monitor_id: "aoc".into(),
            monitor_name: "AOC".into(),
            scale_factor: 1.0,
            relative_x: 0.2,
            relative_y: 0.4,
        });
        let position = target.window_position.clone();
        apply(&mut target, data.preferences);
        assert!(target.autostart);
        assert_eq!(target.window_position, position);
        assert_eq!(target.aliases["app"], vec!["工作"]);
        assert_eq!(target.pinned_apps, vec!["app"]);
    }
}
