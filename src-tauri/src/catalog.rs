use crate::model::AppEntry;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const FULL_INTERVAL: Duration = Duration::from_secs(300);
const NAMES: [&str; 3] = [
    "开始菜单与桌面",
    "安装记录、App Paths 与注册应用",
    "程序目录",
];

#[derive(Clone, Default, Serialize)]
pub struct SourceMetric {
    pub name: String,
    pub duration_ms: u64,
    pub count: usize,
    pub updated_at: u64,
}

#[derive(Clone, Default, Serialize)]
pub struct ScanReport {
    pub revision: u64,
    pub full: bool,
    pub rescanned: Vec<String>,
    pub duration_ms: u64,
    pub time: u64,
    pub app_count: usize,
    pub watched_directories: usize,
    pub watched_registry_keys: usize,
    pub sources: Vec<SourceMetric>,
}

#[derive(Clone, Serialize)]
pub struct ScanResult {
    pub health: Vec<crate::health::EntryHealth>,
    pub apps: Vec<AppEntry>,
    pub report: ScanReport,
}

struct Catalog {
    sources: [Vec<AppEntry>; 3],
    dirty: [bool; 3],
    full_at: Option<Instant>,
    directories: Vec<String>,
    report: ScanReport,
    personal_fingerprint: String,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            sources: Default::default(),
            dirty: [true; 3],
            full_at: None,
            directories: Vec::new(),
            report: ScanReport::default(),
            personal_fingerprint: String::new(),
        }
    }
}

static CATALOG: OnceLock<Mutex<Catalog>> = OnceLock::new();
static SCANNING: Mutex<()> = Mutex::new(());

fn state() -> &'static Mutex<Catalog> {
    CATALOG.get_or_init(Default::default)
}

pub fn invalidate(source: usize) {
    if let Some(dirty) = state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .dirty
        .get_mut(source)
    {
        *dirty = true;
    }
}

pub fn watched(directories: usize, registry: usize) {
    let mut state = state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.report.watched_directories = directories;
    state.report.watched_registry_keys = registry;
}

pub fn report() -> ScanReport {
    state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .report
        .clone()
}

pub fn full_due() -> bool {
    state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .full_at
        .is_some_and(|time| time.elapsed() >= FULL_INTERVAL)
}

fn merged(sources: &[Vec<AppEntry>; 3]) -> Vec<AppEntry> {
    let mut seen = HashSet::new();
    let mut candidates: Vec<_> = sources.iter().flatten().collect();
    candidates.sort_by_key(|app| app.source != "商店 / MSIX 应用");
    let mut apps: Vec<_> = candidates
        .iter()
        .filter(|app| seen.insert(app.id.clone()))
        .map(|app| (**app).clone())
        .collect();
    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    apps
}

pub fn find(id: &str) -> Option<AppEntry> {
    let cfg = crate::config::get();
    if let Some(record) = cfg.entry_records.get(id).filter(|item| item.repaired) {
        return Some(crate::personal::effective(record.entry.clone(), &cfg));
    }
    crate::personal::entries(&cfg)
        .into_iter()
        .find(|app| app.id == id)
        .or_else(|| {
            merged(
                &state()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .sources,
            )
            .iter()
            .find(|app| app.id == id)
            .cloned()
            .map(|app| crate::personal::effective(app, &cfg))
        })
        .or_else(|| {
            cfg.entry_records
                .get(id)
                .map(|record| crate::personal::effective(record.entry.clone(), &cfg))
        })
        .or_else(|| {
            crate::health::decorate(Vec::new(), &cfg)
                .into_iter()
                .find(|app| app.id == id)
        })
}

pub fn all() -> Vec<AppEntry> {
    let cfg = crate::config::get();
    let mut apps = merged(
        &state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sources,
    )
    .into_iter()
    .map(|app| crate::personal::effective(app, &cfg))
    .collect::<Vec<_>>();
    apps.extend(crate::personal::entries(&cfg));
    crate::health::decorate(apps, &cfg)
}

pub fn preferences_changed() {
    state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .report
        .revision += 1;
}

pub fn scan(force: bool) -> ScanResult {
    let _serial = SCANNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let started = Instant::now();
    let cfg = crate::config::get();
    let directories = cfg.extra_dirs.clone();
    let (full, dirty) = {
        let mut state = state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.directories != directories {
            state.dirty[2] = true;
            state.directories = directories;
        }
        let full = force
            || state
                .full_at
                .is_none_or(|time| time.elapsed() >= FULL_INTERVAL);
        let dirty = if full { [true; 3] } else { state.dirty };
        // 取走本轮变更标记。扫描期间新来的事件仍保留到下一轮。
        state.dirty = [false; 3];
        (full, dirty)
    };
    let mut replacements = Vec::new();
    for (source, changed) in dirty.iter().enumerate() {
        if *changed {
            let started = Instant::now();
            let apps = crate::scan_source(source);
            let metric = SourceMetric {
                name: NAMES[source].into(),
                duration_ms: started.elapsed().as_millis() as u64,
                count: apps.len(),
                updated_at: crate::diagnostics::now(),
            };
            replacements.push((source, apps, metric));
        }
    }
    let cfg = crate::config::get();
    let overrides: std::collections::BTreeMap<_, _> = cfg.launch_overrides.iter().collect();
    let records: std::collections::BTreeMap<_, _> = cfg.entry_records.iter().collect();
    let personal_fingerprint =
        serde_json::to_string(&(&cfg.manual_apps, overrides, records)).unwrap_or_default();
    let mut state = state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (source, apps, metric) in replacements {
        state.sources[source] = apps;
        if let Some(existing) = state
            .report
            .sources
            .iter_mut()
            .find(|existing| existing.name == metric.name)
        {
            *existing = metric;
        } else {
            state.report.sources.push(metric);
        }
    }
    if full {
        state.full_at = Some(Instant::now());
    }
    let mut apps: Vec<_> = merged(&state.sources)
        .into_iter()
        .map(|app| crate::personal::effective(app, &cfg))
        .collect();
    apps.extend(crate::personal::entries(&cfg));
    if dirty.iter().any(|changed| *changed) || state.personal_fingerprint != personal_fingerprint {
        state.report.revision += 1;
    }
    state.personal_fingerprint = personal_fingerprint;
    state.report.full = full;
    state.report.rescanned = dirty
        .iter()
        .enumerate()
        .filter(|(_, changed)| **changed)
        .map(|(i, _)| NAMES[i].to_string())
        .collect();
    state.report.duration_ms = started.elapsed().as_millis() as u64;
    state.report.time = crate::diagnostics::now();
    state.report.app_count = apps.len();
    let mut report = state.report.clone();
    drop(state);
    crate::health::remember(&apps);
    let apps = crate::health::decorate(apps, &crate::config::get());
    report.app_count = apps.len();
    let health = crate::health::inspect_all(&apps);
    self::state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .report
        .app_count = apps.len();
    ScanResult {
        apps,
        health,
        report,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_sources_merge_without_losing_real_launch_variants() {
        let path = r"C:\Browser\chrome.exe";
        let bare = AppEntry::new(
            "Google Chrome".into(),
            path.into(),
            "App Paths",
            String::new(),
        );
        let mut shortcut = bare.clone();
        shortcut.arguments = "--source-shortcut-location=start-menu".into();
        shortcut.shortcut_path = Some(r"C:\Start Menu\Google Chrome.lnk".into());
        shortcut.update_identity();
        let mut incognito = shortcut.clone();
        incognito.arguments = "--incognito --source-shortcut-location=desktop".into();
        incognito.update_identity();
        let mut profile = bare.clone();
        profile.arguments = r#"--profile-directory="Profile 1""#.into();
        profile.update_identity();
        let other_install = AppEntry::new(
            "Google Chrome".into(),
            r"D:\Browser\chrome.exe".into(),
            "App Paths",
            String::new(),
        );
        let apps = merged(&[
            vec![shortcut.clone(), incognito, profile],
            vec![bare, other_install],
            vec![],
        ]);
        assert_eq!(apps.len(), 4);
        assert_eq!(
            apps.iter()
                .find(|app| app.id == shortcut.id)
                .unwrap()
                .shortcut_path,
            shortcut.shortcut_path
        );
    }

    #[test]
    fn registered_package_is_preferred_over_a_duplicate_native_entry() {
        let managed = AppEntry::new(
            "App".into(),
            r"shell:AppsFolder\Family_pub!App".into(),
            "商店 / MSIX 应用",
            String::new(),
        );
        let mut native = AppEntry::new(
            "App native".into(),
            r"C:\WindowsApps\Family\app.exe".into(),
            "App Paths",
            String::new(),
        );
        native.id = managed.id.clone();
        let apps = merged(&[vec![native], vec![managed.clone()], vec![]]);
        assert_eq!(apps, vec![managed]);
    }

    #[test]
    #[ignore = "只读本机完整目录，输出审计 JSON；不启动应用或写入用户配置"]
    fn host_discovery_audit() {
        let cfg = crate::config::get();
        let sources = [
            crate::scan_source(0),
            crate::scan_source(1),
            crate::scan_source(2),
        ];
        let mut apps: Vec<_> = merged(&sources)
            .into_iter()
            .map(|app| crate::personal::effective(app, &cfg))
            .collect();
        apps.extend(crate::personal::entries(&cfg));
        let apps = crate::health::decorate(apps, &cfg);
        println!(
            "LUMA_AUDIT={}",
            serde_json::json!({"source_counts":sources.iter().map(Vec::len).collect::<Vec<_>>(),"apps":apps,"hidden_ids":cfg.hidden_apps})
        );
    }

    #[test]
    #[ignore = "读取本机扫描入口；LUMA_EXPECT_APP 与 LUMA_EXPECT_COUNT 指定应用和去重后数量"]
    fn host_discovery_deduplication() {
        let expected = std::env::var("LUMA_EXPECT_APP")
            .expect("需要指定应用")
            .to_lowercase();
        assert!(!expected.is_empty());
        let count: usize = std::env::var("LUMA_EXPECT_COUNT")
            .expect("需要指定数量")
            .parse()
            .unwrap();
        let sources = [crate::scan_source(0), crate::scan_source(1), Vec::new()];
        let apps = merged(&sources);
        let found: Vec<_> = apps
            .iter()
            .filter(|app| app.name.to_lowercase().contains(&expected))
            .collect();
        for app in &found {
            println!(
                "{} | {} | {} | {}",
                app.name, app.path, app.arguments, app.source
            );
        }
        assert_eq!(found.len(), count);
    }
    #[test]
    fn only_invalidated_sources_are_scanned_between_full_refreshes() {
        assert!(scan(true).report.full);
        let cached = scan(false);
        assert!(!cached.report.full && cached.report.rescanned.is_empty());
        invalidate(0);
        let incremental = scan(false);
        assert!(!incremental.report.full);
        assert_eq!(incremental.report.rescanned, vec!["开始菜单与桌面"]);
        state().lock().unwrap().full_at = Some(Instant::now() - Duration::from_secs(301));
        assert!(full_due());
        assert!(scan(false).report.full);
    }
    #[test]
    fn merging_uses_identity_and_keeps_same_name_profiles() {
        let first = AppEntry::new(
            "Editor".into(),
            r"C:\Editor.exe".into(),
            "开始菜单",
            String::new(),
        );
        let mut profile = first.clone();
        profile.arguments = "--profile work".into();
        profile.update_identity();
        let duplicate = first.clone();
        let apps = merged(&[vec![first], vec![duplicate, profile], vec![]]);
        assert_eq!(apps.len(), 2);
        assert_ne!(apps[0].id, apps[1].id);
    }
}
