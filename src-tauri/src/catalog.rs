use crate::model::AppEntry;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const FULL_INTERVAL: Duration = Duration::from_secs(300);
const NAMES: [&str; 3] = ["开始菜单", "安装记录、App Paths 与商店应用", "程序目录"];

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
}

#[derive(Clone, Serialize)]
pub struct ScanResult {
    pub apps: Vec<AppEntry>,
    pub report: ScanReport,
}

struct Catalog {
    sources: [Vec<AppEntry>; 3],
    dirty: [bool; 3],
    full_at: Option<Instant>,
    directories: Vec<String>,
    report: ScanReport,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            sources: Default::default(),
            dirty: [true; 3],
            full_at: None,
            directories: Vec::new(),
            report: ScanReport::default(),
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
    let mut apps: Vec<_> = sources
        .iter()
        .flatten()
        .filter(|app| seen.insert(app.id.clone()))
        .cloned()
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
    state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .sources
        .iter()
        .flatten()
        .find(|app| app.id == id)
        .cloned()
}

pub fn scan(force: bool) -> ScanResult {
    let _serial = SCANNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let started = Instant::now();
    let directories = crate::config::get().extra_dirs;
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
            replacements.push((source, crate::scan_source(source)));
        }
    }
    let mut state = state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (source, apps) in replacements {
        state.sources[source] = apps;
    }
    if full {
        state.full_at = Some(Instant::now());
    }
    let apps = merged(&state.sources);
    if dirty.iter().any(|changed| *changed) {
        state.report.revision += 1;
    }
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
    ScanResult {
        apps,
        report: state.report.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_invalidated_sources_are_scanned_between_full_refreshes() {
        assert!(scan(true).report.full);
        let cached = scan(false);
        assert!(!cached.report.full && cached.report.rescanned.is_empty());
        invalidate(0);
        let incremental = scan(false);
        assert!(!incremental.report.full);
        assert_eq!(incremental.report.rescanned, vec!["开始菜单"]);
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
