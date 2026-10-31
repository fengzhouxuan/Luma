//! Luma 配置存储层
//!
//! 配置文件位于 `%APPDATA%\Luma\config.json`，以 JSON 存储。
//! 注意：Luma 以管理员权限运行，写入 APPDATA 时需确保路径权限正确。

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
    Combo {
        modifiers: Vec<String>,
        key: String,
    },
}

/// 窗口外观配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Appearance {
    /// 背景不透明度 0.0~1.0
    pub opacity: f64,
    /// 圆角大小（px）
    pub corner_radius: f64,
}

/// 完整配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub hotkey: Hotkey,
    pub autostart: bool,
    pub appearance: Appearance,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: Hotkey::Double {
                key: "alt".to_string(),
            },
            autostart: false,
            appearance: Appearance {
                opacity: 0.8,
                corner_radius: 12.0,
            },
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
    std::fs::write(config_path(), text)
}

/// 保存配置并更新内存缓存
pub fn save(cfg: Config) -> std::io::Result<()> {
    save_inner(&cfg)?;
    // 更新内存缓存
    if let Some(m) = CONFIG.get() {
        if let Ok(mut guard) = m.lock() {
            *guard = cfg;
        }
    }
    Ok(())
}

/// 获取当前配置（内存缓存，避免频繁读盘）
pub fn get() -> Config {
    let m = CONFIG.get_or_init(|| Mutex::new(load()));
    m.lock().map(|g| g.clone()).unwrap_or_default()
}

/// 初始化（启动时调用一次）
pub fn init() {
    let mut cfg = load();
    // 同步实际的自启状态（计划任务是否启用）
    #[cfg(windows)]
    {
        if let Ok(output) = std::process::Command::new("schtasks")
            .args(["/Query", "/TN", "LaunchpadElevated", "/FO", "CSV", "/NH"])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            // CSV 格式最后一列是状态，可能为 "Ready"/"Running"（启用）或 "Disabled"（禁用）
            // 中英文系统都通过是否含 "Disabled"/"禁用" 判断
            let enabled = output.status.success()
                && !text.contains("Disabled")
                && !text.contains("禁用");
            cfg.autostart = enabled;
        }
    }
    let _ = save_inner(&cfg);
    let _ = CONFIG.set(Mutex::new(cfg));
}

// ==================== Tauri 命令 ====================

#[tauri::command]
pub fn get_config() -> Config {
    get()
}

#[tauri::command]
pub fn set_config(cfg: Config) -> Result<(), String> {
    save(cfg).map_err(|e| format!("保存配置失败: {}", e))
}

/// 设置开机自启（通过计划任务 LaunchpadElevated 的启用/禁用实现）
#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let action = if enabled { "/Enable" } else { "/Disable" };
        let output = std::process::Command::new("schtasks")
            .args(["/Change", "/TN", "LaunchpadElevated", action])
            .output()
            .map_err(|e| format!("执行 schtasks 失败: {}", e))?;

        if output.status.success() {
            // 同步更新配置里的 autostart 字段
            let mut cfg = get();
            cfg.autostart = enabled;
            let _ = save(cfg);
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("设置自启失败: {}", stderr.trim()))
        }
    }

    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err("当前平台不支持".to_string())
    }
}
