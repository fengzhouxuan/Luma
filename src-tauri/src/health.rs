//! Local checks never launch entries or probe websites/network shares.
use crate::{
    catalog,
    config::{self, Config},
    model::{AppEntry, EntryKind},
    personal::ManualApp,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EntryRecord {
    pub entry: AppEntry,
    #[serde(default)]
    pub repaired: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct EntryHealth {
    pub id: String,
    pub state: &'static str,
    pub message: String,
}

fn referenced(cfg: &Config, id: &str) -> bool {
    cfg.pinned_apps
        .iter()
        .chain(cfg.hidden_apps.iter())
        .any(|key| key == id)
        || cfg.aliases.contains_key(id)
        || cfg.app_categories.contains_key(id)
        || cfg.launch_overrides.contains_key(id)
}
pub fn retain(cfg: &mut Config, app: &AppEntry) {
    if cfg.manual_apps.iter().any(|item| item.id == app.id)
        || cfg
            .entry_records
            .get(&app.id)
            .is_some_and(|value| value.repaired)
    {
        return;
    }
    let mut entry = app.clone();
    entry.icon = None;
    cfg.entry_records.insert(
        app.id.clone(),
        EntryRecord {
            entry,
            repaired: false,
        },
    );
}
pub fn remember(apps: &[AppEntry]) {
    // Host discovery tests read installed applications but must never write user settings.
    if cfg!(test) {
        return;
    }
    let cfg = config::get();
    let mut next = cfg.clone();
    for app in apps {
        if referenced(&cfg, &app.id) {
            retain(&mut next, app);
        }
    }
    next.entry_records
        .retain(|id, item| item.repaired || referenced(&cfg, id));
    if next.entry_records == cfg.entry_records {
        return;
    }
    if let Err(error) = config::update(|current| {
        for app in apps {
            if referenced(current, &app.id) {
                retain(current, app);
            }
        }
        let keep: HashSet<_> = current
            .entry_records
            .iter()
            .filter(|(id, item)| item.repaired || referenced(current, id))
            .map(|(id, _)| id.clone())
            .collect();
        current.entry_records.retain(|id, _| keep.contains(id));
    }) {
        crate::diagnostics::record("入口检查", "保留入口", None, error);
    }
}
pub fn decorate(mut apps: Vec<AppEntry>, cfg: &Config) -> Vec<AppEntry> {
    for app in &mut apps {
        if let Some(record) = cfg.entry_records.get(&app.id).filter(|item| item.repaired) {
            *app = crate::personal::effective(record.entry.clone(), cfg);
        }
    }
    let mut seen: HashSet<_> = apps.iter().map(|app| app.id.clone()).collect();
    for (id, record) in &cfg.entry_records {
        if (record.repaired || referenced(cfg, id)) && seen.insert(id.clone()) {
            apps.push(crate::personal::effective(record.entry.clone(), cfg));
        }
    }
    // Old pins encode their target and arguments, so they remain repairable after upgrading.
    for id in &cfg.pinned_apps {
        if seen.contains(id) {
            continue;
        }
        if let Ok((path, arguments)) = serde_json::from_str::<(String, String)>(id) {
            if !Path::new(&path).is_absolute() && !managed(&path) {
                continue;
            }
            let name = Path::new(&path)
                .file_stem()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "未发现的应用".into());
            let mut app = AppEntry::new(name, path, "保留入口", String::new());
            app.id = id.clone();
            app.arguments = arguments;
            app.hidden_reason = None;
            apps.push(crate::personal::effective(app, cfg));
            seen.insert(id.clone());
        }
    }
    apps
}
fn managed(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("shell:") || lower.starts_with("::{") || path.contains("://")
}
fn network(path: &str) -> bool {
    let path = path.replace('/', "\\");
    if path.starts_with(r"\\") && !path.starts_with(r"\\?\")
        || path.to_ascii_lowercase().starts_with(r"\\?\unc\")
    {
        return true;
    }
    #[cfg(windows)]
    {
        let normal = path.strip_prefix(r"\\?\").unwrap_or(&path);
        let bytes = normal.as_bytes();
        if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            let root = [bytes[0] as u16, b':' as u16, b'\\' as u16, 0];
            // DRIVE_REMOTE, queried without accessing files on the mapped drive.
            return unsafe {
                windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root.as_ptr())
            } == 4;
        }
    }
    false
}

pub fn inspect(app: &AppEntry) -> Option<EntryHealth> {
    inspect_launch(app, false)
}
fn inspect_launch(app: &AppEntry, direct: bool) -> Option<EntryHealth> {
    let result = |state, message: &str| {
        Some(EntryHealth {
            id: app.id.clone(),
            state,
            message: message.into(),
        })
    };
    if app.kind == EntryKind::Web {
        return crate::personal::validate_target(app.kind, &app.path, false)
            .err()
            .map(|message| EntryHealth {
                id: app.id.clone(),
                state: "missing",
                message,
            });
    }
    if managed(&app.path) || (!direct && app.shortcut_path.as_deref().is_some_and(managed)) {
        return result("unchecked", "由 Windows 或协议处理程序管理，未执行启动验证");
    }
    if network(&app.path)
        || app.shortcut_path.as_deref().is_some_and(network)
        || app.working_dir.as_deref().is_some_and(network)
    {
        return result("unchecked", "网络路径在启动时验证，未在后台访问");
    }
    let target = Path::new(&app.path);
    let exists = match std::fs::metadata(target) {
        Ok(metadata) => {
            if app.kind == EntryKind::Folder {
                metadata.is_dir()
            } else {
                metadata.is_file()
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return result("warning", "暂时无法访问目标，请检查设备连接或访问权限"),
    };
    let shortcut = app.shortcut_path.as_deref().filter(|path| !managed(path));
    if !exists {
        if shortcut.is_some_and(|path| Path::new(path).is_file()) {
            return result(
                "warning",
                "目标文件未发现；原快捷方式仍存在，可重新选择目标",
            );
        }
        return result("missing", "入口目标不存在或类型不匹配，请重新选择目标");
    }
    if shortcut.is_some_and(|path| !Path::new(path).is_file()) {
        return result("warning", "原快捷方式已失效，启动时将使用目标文件");
    }
    if (direct || !shortcut.is_some_and(|path| Path::new(path).is_file()))
        && app
            .working_dir
            .as_deref()
            .is_some_and(|path| !Path::new(path).is_dir())
    {
        return result("warning", "工作目录不存在，启动时将回退到目标所在目录");
    }
    None
}
pub fn inspect_all(apps: &[AppEntry]) -> Vec<EntryHealth> {
    let cfg = config::get();
    apps.iter()
        .filter_map(|app| inspect_launch(app, cfg.launch_overrides.contains_key(&app.id)))
        .collect()
}

pub fn validate_record(id: &str, record: &EntryRecord) -> Result<(), String> {
    let item = &record.entry;
    if id != item.id
        || id.len() > 32768
        || id.contains('\0')
        || item.name.trim().is_empty()
        || item.name.len() > 256
        || item.path.len() > 32768
        || item.path.contains('\0')
        || item.arguments.len() > 32768
        || item.arguments.contains('\0')
    {
        return Err("保留入口字段无效".into());
    }
    if item.kind != EntryKind::App {
        crate::personal::validate_target(item.kind, &item.path, false)?;
    } else if !managed(&item.path) && !Path::new(&item.path).is_absolute() {
        return Err("保留入口目标必须是绝对路径".into());
    }
    if item
        .shortcut_path
        .as_deref()
        .is_some_and(|path| path.len() > 32768 || path.contains('\0'))
    {
        return Err("快捷方式路径无效".into());
    }
    if item
        .working_dir
        .as_deref()
        .is_some_and(|path| path.len() > 32768 || path.contains('\0'))
    {
        return Err("工作目录字段无效".into());
    }
    if record.repaired {
        crate::personal::validate(
            &ManualApp {
                id: "manual-validation".into(),
                name: item.name.clone(),
                kind: item.kind,
                path: item.path.clone(),
                arguments: item.arguments.clone(),
                working_dir: item.working_dir.clone(),
                shortcut_path: item.shortcut_path.clone(),
            },
            false,
        )?;
    }
    Ok(())
}

#[tauri::command]
pub async fn check_entry_health() -> Result<Vec<EntryHealth>, String> {
    tauri::async_runtime::spawn_blocking(|| inspect_all(&catalog::all()))
        .await
        .map_err(|error| error.to_string())
}
#[tauri::command]
pub fn repair_entry(
    app_id: String,
    mut target: ManualApp,
    aliases: Vec<String>,
) -> Result<Config, String> {
    let original = catalog::find(&app_id).ok_or("入口已不存在，请刷新后重试")?;
    target.name = original.name.clone();
    if target.kind != original.kind {
        return Err("请选择与原入口相同类型的目标".into());
    }
    crate::personal::validate(&target, true)?;
    let aliases = crate::personal::validate_aliases(aliases)?;
    if config::get()
        .manual_apps
        .iter()
        .any(|item| item.id == app_id)
    {
        target.id = app_id;
        return crate::personal::save_manual_app(target, aliases);
    }
    let mut replacement = original.clone();
    replacement.path = target.path;
    replacement.shortcut_path = target.shortcut_path;
    replacement.arguments = target.arguments;
    replacement.working_dir = target.working_dir;
    replacement.icon = None;
    replacement.hidden_reason = None;
    replacement.source = "修复入口".into();
    replacement.source_path = replacement.path.clone();
    config::update(|cfg| apply_repair(cfg, replacement, aliases))?;
    catalog::preferences_changed();
    Ok(config::get())
}

fn apply_repair(cfg: &mut Config, mut replacement: AppEntry, aliases: Vec<String>) {
    let app_id = replacement.id.clone();
    let kind = replacement.kind;
    let requested = crate::personal::LaunchOverride {
        arguments: replacement.arguments.clone(),
        working_dir: replacement.working_dir.clone(),
    };
    if matches!(kind, EntryKind::App | EntryKind::Script) {
        let baseline = replacement
            .shortcut_path
            .as_ref()
            .and_then(|path| crate::parse_lnk(&path.into()));
        replacement.arguments = baseline
            .as_ref()
            .map(|app| app.arguments.clone())
            .unwrap_or_default();
        replacement.working_dir = baseline
            .and_then(|app| app.working_dir)
            .filter(|directory| Path::new(directory).is_absolute());
    }
    let customized = requested.arguments != replacement.arguments
        || requested.working_dir != replacement.working_dir;
    cfg.entry_records.insert(
        app_id.clone(),
        EntryRecord {
            entry: replacement,
            repaired: true,
        },
    );
    cfg.aliases.insert(app_id.clone(), aliases);
    // Keep the new target's defaults separate so clearing custom launch options works.
    if customized && matches!(kind, EntryKind::App | EntryKind::Script) {
        cfg.launch_overrides.insert(app_id.clone(), requested);
    } else {
        cfg.launch_overrides.remove(&app_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> AppEntry {
        AppEntry::new(
            "原应用".into(),
            r"Z:\luma-missing-qa\old.exe".into(),
            "测试",
            String::new(),
        )
    }
    #[test]
    fn health_distinguishes_missing_files_from_managed_web_and_network_targets() {
        let mut entry = app();
        assert_eq!(inspect(&entry).unwrap().state, "missing");
        for path in [r"\\server\share\app.exe", r"\\?\UNC\server\share\app.exe"] {
            entry.path = path.into();
            assert_eq!(inspect(&entry).unwrap().state, "unchecked");
        }
        entry.path = "shell:AppsFolder\\Example!App".into();
        assert_eq!(inspect(&entry).unwrap().state, "unchecked");
        entry.path = r"C:\Missing\managed-app.exe".into();
        entry.shortcut_path = Some("shell:AppsFolder\\Example!App".into());
        assert_eq!(inspect_launch(&entry, false).unwrap().state, "unchecked");
        assert_eq!(inspect_launch(&entry, true).unwrap().state, "missing");
        entry.shortcut_path = None;
        entry.kind = EntryKind::Web;
        entry.path = "https://example.com/".into();
        assert!(inspect(&entry).is_none());
        entry.path = "file:///C:/test".into();
        assert_eq!(inspect(&entry).unwrap().state, "missing");
    }
    #[test]
    fn repaired_entries_keep_identity_preferences_and_survive_discovery_changes() {
        let original = app();
        let id = original.id.clone();
        let mut cfg = Config::default();
        cfg.pinned_apps = vec!["other".into(), id.clone()];
        cfg.app_categories.insert(id.clone(), "办公".into());
        cfg.usage.insert(
            id.clone(),
            config::Usage {
                count: 7,
                last_launch: 123,
            },
        );
        retain(&mut cfg, &original);
        let mut repaired = original.clone();
        repaired.path = r"C:\New location\new.exe".into();
        repaired.arguments = "--profile work".into();
        apply_repair(&mut cfg, repaired, vec!["工作".into()]);
        let restored = decorate(vec![], &cfg);
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, id);
        assert_eq!(restored[0].name, original.name);
        assert_eq!(restored[0].path, r"C:\New location\new.exe");
        assert_eq!(restored[0].arguments, "--profile work");
        assert_eq!(cfg.pinned_apps, vec!["other".to_string(), id.clone()]);
        assert_eq!(cfg.app_categories[&id], "办公");
        assert_eq!(cfg.aliases[&id], vec!["工作"]);
        assert_eq!(cfg.usage[&id].count, 7);
        assert_eq!(decorate(vec![original], &cfg).len(), 1);
        let mut changed = restored[0].clone();
        changed.path = r"C:\Old scanner\old.exe".into();
        retain(&mut cfg, &changed);
        assert_eq!(
            cfg.entry_records[&id].entry.path,
            r"C:\New location\new.exe"
        );
    }
    #[test]
    fn old_pins_can_be_repaired_even_without_saved_metadata() {
        let mut cfg = Config::default();
        let entry = app();
        cfg.pinned_apps.push(entry.id.clone());
        let restored = decorate(vec![], &cfg);
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, entry.id);
        assert_eq!(inspect(&restored[0]).unwrap().state, "missing");
        let mut record = EntryRecord {
            entry,
            repaired: true,
        };
        assert!(validate_record(&record.entry.id, &record).is_ok());
        record.entry.shortcut_path = Some("bad\0shortcut".into());
        assert!(validate_record(&record.entry.id, &record).is_err());
    }
    #[test]
    fn repaired_entry_custom_launch_can_be_cleared_to_the_new_targets_defaults() {
        let mut cfg = Config::default();
        let mut entry = app();
        let id = entry.id.clone();
        entry.arguments = "--profile work".into();
        entry.working_dir = Some(r"C:\Work".into());
        apply_repair(&mut cfg, entry, vec![]);
        assert_eq!(
            crate::personal::effective(cfg.entry_records[&id].entry.clone(), &cfg).arguments,
            "--profile work"
        );
        cfg.launch_overrides.remove(&id);
        let restored = crate::personal::effective(cfg.entry_records[&id].entry.clone(), &cfg);
        assert!(restored.arguments.is_empty());
        assert!(restored.working_dir.is_none());
    }
    #[test]
    fn local_folder_and_shortcut_fallback_are_checked_by_type() {
        let dir = std::env::temp_dir();
        let mut entry = app();
        entry.path = dir.to_string_lossy().into();
        entry.kind = EntryKind::Folder;
        assert!(inspect(&entry).is_none());
        entry.kind = EntryKind::App;
        assert_eq!(inspect(&entry).unwrap().state, "missing");
        entry.path = std::env::current_exe().unwrap().to_string_lossy().into();
        entry.shortcut_path = Some(r"Z:\luma-missing-qa\gone.lnk".into());
        assert_eq!(inspect(&entry).unwrap().state, "warning");
        entry.shortcut_path = Some(entry.path.clone());
        entry.working_dir = Some("%HOMEDRIVE%%HOMEPATH%".into());
        assert!(inspect_launch(&entry, false).is_none());
        assert_eq!(inspect_launch(&entry, true).unwrap().state, "warning");
    }
    #[test]
    fn shell_owned_snapshot_metadata_round_trips_without_restricting_document_targets() {
        let mut entry = app();
        entry.path = r"C:\Program Files\Git\ReleaseNotes.html".into();
        entry.working_dir = Some("%HOMEDRIVE%%HOMEPATH%".into());
        let mut record = EntryRecord {
            entry,
            repaired: false,
        };
        assert!(validate_record(&record.entry.id, &record).is_ok());
        record.repaired = true;
        assert!(validate_record(&record.entry.id, &record).is_err());
    }
}
