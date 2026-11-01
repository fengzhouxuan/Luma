use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Serialize)]
pub struct Event {
    pub time: u64,
    pub stage: String,
    pub path: String,
    pub app_id: Option<String>,
    pub message: String,
}

static EVENTS: OnceLock<Mutex<VecDeque<Event>>> = OnceLock::new();

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn record(stage: &str, path: &str, app_id: Option<&str>, message: impl Into<String>) {
    let mut events = EVENTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    events.retain(|event| {
        !(event.stage == stage && event.path == path && event.app_id.as_deref() == app_id)
    });
    events.push_front(Event {
        time: now(),
        stage: stage.into(),
        path: path.into(),
        app_id: app_id.map(str::to_string),
        message: message.into(),
    });
    events.truncate(256);
}

#[derive(Serialize)]
pub struct Snapshot {
    pub version: String,
    pub executable: String,
    pub report: crate::catalog::ScanReport,
    pub events: Vec<Event>,
    pub icons: crate::icons::Metrics,
}

#[tauri::command]
pub fn get_diagnostics() -> Snapshot {
    Snapshot {
        version: env!("CARGO_PKG_VERSION").into(),
        executable: std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        report: crate::catalog::report(),
        icons: crate::icons::metrics(),
        events: EVENTS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect(),
    }
}
