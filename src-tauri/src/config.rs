//! Luma 配置存储层
//!
//! 配置文件位于 `%APPDATA%\Luma\config.json`，以 JSON 存储。
//! 自启和个人配置使用当前用户目录。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;

/// 唤醒快捷键配置
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Hotkey {
    /// 双击单键：如双击 Alt / Ctrl / Shift
    Double { key: String },
    /// 组合键：修饰键 + 主键，如 Ctrl+Space
    Combo { modifiers: Vec<String>, key: String },
}

/// 窗口外观配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Appearance {
    #[serde(default)]
    pub density: Density,
    /// 背景不透明度 0.0~1.0
    pub opacity: f64,
    /// 圆角大小（px）
    pub corner_radius: f64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    #[default]
    Standard,
    Comfortable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub count: u64,
    pub last_launch: u64,
}

/// 完整配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub window_size: crate::position::WindowSize,
    #[serde(default)]
    pub entry_records: std::collections::HashMap<String, crate::health::EntryRecord>,
    pub hotkey: Hotkey,
    pub autostart: bool,
    pub appearance: Appearance,
    /// 是否显示全部应用（包括被自动过滤的噪音应用）
    #[serde(default)]
    pub show_all: bool,
    /// 额外扫描目录（绿色版游戏等未注册到开始菜单的程序所在目录）
    #[serde(default)]
    pub extra_dirs: Vec<String>,
    /// 自定义分类列表（用户手动创建的分类名）
    #[serde(default)]
    pub custom_categories: Vec<String>,
    /// 应用手动分类映射：应用名 → 分类名（覆盖自动分类）
    #[serde(default)]
    pub app_categories: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub pinned_apps: Vec<String>,
    #[serde(default)]
    pub hidden_apps: Vec<String>,
    #[serde(default)]
    pub usage: std::collections::HashMap<String, Usage>,
    /// 仅由用户主动拖动更新；自动断屏回退不覆盖。
    #[serde(default)]
    pub window_position: Option<crate::position::SavedPosition>,
    #[serde(default)]
    pub manual_apps: Vec<crate::personal::ManualApp>,
    #[serde(default)]
    pub aliases: std::collections::HashMap<String, Vec<String>>,
    #[serde(default)]
    pub launch_overrides: std::collections::HashMap<String, crate::personal::LaunchOverride>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            window_size: Default::default(),
            entry_records: Default::default(),
            hotkey: Hotkey::Combo {
                modifiers: vec!["alt".to_string(), "shift".to_string()],
                key: "space".to_string(),
            },
            autostart: false,
            appearance: Appearance {
                opacity: 0.8,
                corner_radius: 24.0,
                density: Density::Standard,
            },
            show_all: false,
            extra_dirs: Vec::new(),
            custom_categories: Vec::new(),
            app_categories: std::collections::HashMap::new(),
            pinned_apps: Vec::new(),
            hidden_apps: Vec::new(),
            usage: Default::default(),
            window_position: None,
            manual_apps: Vec::new(),
            aliases: Default::default(),
            launch_overrides: Default::default(),
        }
    }
}

/// 全局配置（线程安全，Atomic 风格）
static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();

fn config_dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join("Luma")
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// 加载配置：优先读文件，失败则用默认值并写回
pub fn load() -> Config {
    let path = config_path();
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<Config>(&text) {
            // 迁移旧配置：
            // 1. 双击单键模式已废弃，重置为默认组合键
            // 2. 旧默认键 Alt+Space 与 Windows 系统菜单冲突，迁移到 Alt+Shift+Space
            let needs_migrate = matches!(cfg.hotkey, Hotkey::Double { .. })
                || matches!(
                    &cfg.hotkey,
                    Hotkey::Combo { modifiers, key }
                        if modifiers.len() == 1 && modifiers[0] == "alt" && key == "space"
                );
            if needs_migrate {
                let mut migrated = cfg;
                migrated.hotkey = Hotkey::Combo {
                    modifiers: vec!["alt".to_string(), "shift".to_string()],
                    key: "space".to_string(),
                };
                let _ = save_inner(&migrated);
                return migrated;
            }
            return cfg;
        }
    }
    let cfg = Config::default();
    let _ = save_inner(&cfg);
    cfg
}

fn save_inner(cfg: &Config) -> std::io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let text = serde_json::to_string_pretty(cfg).unwrap_or_else(|_| "{}".to_string());
    let temp = dir.join(format!("config-{}.tmp", std::process::id()));
    std::fs::write(&temp, text)?;
    let result = std::fs::rename(&temp, config_path());
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

/// 保存配置并更新内存缓存
pub fn update<T>(change: impl FnOnce(&mut Config) -> T) -> Result<T, String> {
    let mut guard = CONFIG
        .get_or_init(|| Mutex::new(load()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut next = guard.clone();
    let value = change(&mut next);
    save_inner(&next).map_err(|e| format!("保存配置失败: {e}"))?;
    *guard = next;
    Ok(value)
}

/// 获取当前配置（内存缓存，避免频繁读盘）
pub fn get() -> Config {
    let m = CONFIG.get_or_init(|| Mutex::new(load()));
    m.lock().map(|g| g.clone()).unwrap_or_default()
}

/// 初始化（启动时调用一次）
pub fn init() {
    let mut cfg = load();
    cfg.autostart = crate::autostart::sync();
    let _ = save_inner(&cfg);
    let _ = CONFIG.set(Mutex::new(cfg));
}

// ==================== Tauri 命令 ====================

#[tauri::command]
pub fn get_config() -> Config {
    get()
}

#[tauri::command]
pub fn set_appearance(appearance: Appearance, app: tauri::AppHandle) -> Result<(), String> {
    if !appearance.opacity.is_finite()
        || !(0.3..=0.95).contains(&appearance.opacity)
        || !appearance.corner_radius.is_finite()
        || !(0.0..=48.0).contains(&appearance.corner_radius)
    {
        return Err("外观设置超出有效范围".into());
    }
    update(|current| current.appearance = appearance.clone())?;
    crate::appearance::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn set_config(cfg: Config, app: tauri::AppHandle) -> Result<(), String> {
    // 外观和热键保存不能覆盖同时更新的置顶、分类或使用记录。
    update(|current| {
        current.hotkey = cfg.hotkey.clone();
        current.appearance = cfg.appearance.clone();
    })?;
    crate::appearance::refresh(&app);

    // 快捷键变更后重新注册系统热键
    #[cfg(windows)]
    {
        if let Err(e) = crate::double_alt::register(&cfg.hotkey) {
            // 注册失败不阻断保存（配置已落盘），但返回警告信息
            eprintln!("[luma] {}", e);
        }
    }

    Ok(())
}

/// 设置"显示全部"开关（是否显示被自动过滤的噪音应用）
#[tauri::command]
pub fn set_show_all(show_all: bool) -> Result<(), String> {
    update(|cfg| cfg.show_all = show_all)
}

/// 设置额外扫描目录（绿色版游戏等）
#[tauri::command]
pub fn set_extra_dirs(dirs: Vec<String>) -> Result<(), String> {
    update(|cfg| cfg.extra_dirs = dirs)?;
    crate::catalog::invalidate(2);
    Ok(())
}

/// 手动设置某个应用的分类（覆盖自动分类）
#[tauri::command]
pub fn set_app_category(app_id: String, category: String) -> Result<(), String> {
    let entry = crate::catalog::find(&app_id);
    let cfg = get();
    validate_category(&category, &cfg.custom_categories)?;
    // 空值明确表示使用自动分类，并覆盖旧版按名称保存的映射。
    update(|cfg| {
        if let Some(entry) = &entry {
            crate::health::retain(cfg, entry);
        }
        cfg.app_categories.insert(app_id, category);
    })
}

fn validate_category(category: &str, custom: &[String]) -> Result<(), String> {
    if ["", "游戏", "办公", "开发", "媒体", "社交", "浏览器", "其他"].contains(&category)
        || custom.iter().any(|name| name == category)
    {
        Ok(())
    } else {
        Err("分类已不存在，请重新选择".into())
    }
}

/// 添加自定义分类
#[tauri::command]
pub fn add_custom_category(name: String) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty()
        || [
            "全部",
            "游戏",
            "办公",
            "开发",
            "媒体",
            "社交",
            "浏览器",
            "其他",
        ]
        .contains(&name.as_str())
    {
        return Err("分类名称为空或已被系统分类使用".into());
    }
    update(|cfg| {
        if !cfg.custom_categories.contains(&name) {
            cfg.custom_categories.push(name);
        }
    })
}

/// 删除自定义分类
#[tauri::command]
pub fn remove_custom_category(name: String) -> Result<(), String> {
    update(|cfg| {
        cfg.custom_categories.retain(|c| c != &name);
        cfg.app_categories.retain(|_, cat| cat != &name);
    })
}

#[tauri::command]
pub fn set_app_pinned(app_id: String, pinned: bool) -> Result<Config, String> {
    let entry = crate::catalog::find(&app_id);
    update(|cfg| {
        if pinned {
            if let Some(entry) = &entry {
                crate::health::retain(cfg, entry);
            }
        }
        cfg.pinned_apps.retain(|id| id != &app_id);
        if pinned {
            cfg.pinned_apps.push(app_id);
        }
        cfg.clone()
    })
}

#[tauri::command]
pub fn place_pinned_app(app_id: String, before_id: Option<String>) -> Result<Config, String> {
    let entry = crate::catalog::find(&app_id);
    update(|cfg| {
        if let Some(entry) = &entry {
            crate::health::retain(cfg, entry);
        }
        place_pin(&mut cfg.pinned_apps, app_id, before_id.as_deref());
        cfg.clone()
    })
}

fn place_pin(pins: &mut Vec<String>, app_id: String, before_id: Option<&str>) {
    if before_id == Some(app_id.as_str()) && pins.contains(&app_id) {
        return;
    }
    pins.retain(|id| id != &app_id);
    let index = before_id
        .and_then(|before| pins.iter().position(|id| id == before))
        .unwrap_or(pins.len());
    pins.insert(index, app_id);
}

#[tauri::command]
pub fn set_app_hidden(app_id: String, hidden: bool) -> Result<Config, String> {
    update(|cfg| {
        cfg.hidden_apps.retain(|id| id != &app_id);
        if hidden {
            cfg.hidden_apps.push(app_id);
        }
        cfg.clone()
    })
}

pub fn record_launch(app_id: &str) -> Result<Usage, String> {
    update(|cfg| increment_usage(cfg, app_id, crate::diagnostics::now()))
}

fn increment_usage(cfg: &mut Config, id: &str, now: u64) -> Usage {
    let entry = cfg.usage.entry(id.to_string()).or_default();
    entry.count = entry.count.saturating_add(1);
    entry.last_launch = now;
    entry.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn category_drop_accepts_builtin_custom_and_auto_but_rejects_stale_targets() {
        assert!(validate_category("", &[]).is_ok());
        assert!(validate_category("办公", &[]).is_ok());
        assert!(validate_category("常用", &["常用".into()]).is_ok());
        assert!(validate_category("已删除", &[]).is_err());
        assert!(validate_category("全部", &[]).is_err());
    }
    #[test]
    fn placing_pins_adds_moves_and_keeps_other_entries_in_order() {
        let mut pins = vec!["a".into(), "missing".into(), "b".into(), "c".into()];
        place_pin(&mut pins, "c".into(), Some("a"));
        assert_eq!(pins, ["c", "a", "missing", "b"]);
        place_pin(&mut pins, "a".into(), None);
        assert_eq!(pins, ["c", "missing", "b", "a"]);
        place_pin(&mut pins, "new".into(), Some("b"));
        assert_eq!(pins, ["c", "missing", "new", "b", "a"]);
        place_pin(&mut pins, "new".into(), Some("new"));
        assert_eq!(pins, ["c", "missing", "new", "b", "a"]);
        place_pin(&mut pins, "b".into(), Some("removed"));
        assert_eq!(pins, ["c", "missing", "new", "a", "b"]);
    }
    #[test]
    fn legacy_config_gets_defaults_for_new_preferences() {
        let mut old = serde_json::to_value(Config::default()).unwrap();
        for field in [
            "usage",
            "pinned_apps",
            "hidden_apps",
            "window_position",
            "window_size",
            "entry_records",
            "manual_apps",
            "aliases",
            "launch_overrides",
        ] {
            old.as_object_mut().unwrap().remove(field);
        }
        old["appearance"].as_object_mut().unwrap().remove("density");
        let cfg: Config = serde_json::from_value(old).unwrap();
        assert_eq!(cfg.window_size, crate::position::WindowSize::default());
        assert_eq!(cfg.appearance.density, Density::Standard);
        assert!(cfg.entry_records.is_empty());
        assert!(cfg.pinned_apps.is_empty() && cfg.hidden_apps.is_empty() && cfg.usage.is_empty());
        assert!(cfg.window_position.is_none());
        assert!(
            cfg.manual_apps.is_empty() && cfg.aliases.is_empty() && cfg.launch_overrides.is_empty()
        );
    }
    #[test]
    fn usage_is_per_identity_and_counts_successful_launches() {
        let mut cfg = Config::default();
        assert_eq!(increment_usage(&mut cfg, "one", 10).count, 1);
        assert_eq!(increment_usage(&mut cfg, "two", 11).count, 1);
        let entry = increment_usage(&mut cfg, "one", 12);
        assert_eq!((entry.count, entry.last_launch), (2, 12));
    }
}
