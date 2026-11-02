//! Everything is the index owner. Queries run in a disposable copy of this executable,
//! before Tauri starts, so a hung IPC server can be cancelled without leaking threads.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[cfg(windows)]
mod sdk;

const WORKER_ARG: &str = "--luma-file-search-worker";
const PAGE_SIZE: u32 = 50;
const MAX_OUTPUT: u64 = 16 * 1024 * 1024;

#[tauri::command]
pub fn open_everything_download(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url("https://www.voidtools.com/downloads/", None::<&str>)
        .map_err(|error| error.to_string())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

impl FileEntry {
    fn new(path: String, is_dir: bool, size: Option<u64>) -> Self {
        let id = format!("{:x}", Sha256::digest(path.as_bytes()));
        let name = path
            .rsplit(['\\', '/'])
            .find(|part| !part.is_empty())
            .unwrap_or(&path)
            .to_owned();
        Self {
            id,
            name,
            path,
            is_dir,
            size,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SearchResult {
    pub state: String,
    pub message: String,
    pub version: Option<String>,
    pub backend: Option<String>,
    pub total: u64,
    pub offset: u32,
    pub items: Vec<FileEntry>,
}

impl SearchResult {
    fn status(state: &str, message: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            message: message.into(),
            version: None,
            backend: None,
            total: 0,
            offset: 0,
            items: vec![],
        }
    }
}

#[derive(Deserialize, Serialize)]
struct Request {
    query: String,
    offset: u32,
    limit: u32,
    // Used by the optional live compatibility test, never exposed to frontend IPC.
    #[serde(default)]
    force_legacy: bool,
}

fn validate_request(request: &Request) -> Result<(), String> {
    if request.query.chars().count() > 1024 || request.query.contains('\0') {
        return Err("搜索词过长或包含无效字符".into());
    }
    if request.limit == 0 || request.limit > PAGE_SIZE || request.offset > 100_000 {
        return Err("搜索分页参数无效".into());
    }
    Ok(())
}

/// Worker mode must run before the single-instance plugin and before creating windows.
pub fn run_file_search_worker() -> bool {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(WORKER_ARG)) {
        return false;
    }
    let mut input = String::new();
    let result = (|| {
        std::io::stdin()
            .take(8192)
            .read_to_string(&mut input)
            .map_err(|e| e.to_string())?;
        let request: Request = serde_json::from_str(&input).map_err(|e| e.to_string())?;
        validate_request(&request)?;
        #[cfg(windows)]
        {
            Ok::<_, String>(sdk::search(&request))
        }
        #[cfg(not(windows))]
        {
            Ok::<_, String>(SearchResult::status(
                "unavailable",
                "文件搜索仅支持 Windows",
            ))
        }
    })()
    .unwrap_or_else(|error| SearchResult::status("error", error));
    if let Ok(bytes) = serde_json::to_vec(&result) {
        let _ = std::io::stdout().write_all(&bytes);
    }
    true
}

struct Active {
    id: String,
    cancelled: Arc<AtomicBool>,
}
fn active() -> &'static Mutex<Option<Active>> {
    static VALUE: OnceLock<Mutex<Option<Active>>> = OnceLock::new();
    VALUE.get_or_init(|| Mutex::new(None))
}
fn results() -> &'static Mutex<HashMap<String, FileEntry>> {
    static VALUE: OnceLock<Mutex<HashMap<String, FileEntry>>> = OnceLock::new();
    VALUE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn cached_entry(id: &str) -> Result<FileEntry, String> {
    results()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
        .ok_or_else(|| "搜索结果已过期，请重新搜索".into())
}

#[tauri::command]
pub fn cancel_file_search(request_id: String) {
    let guard = active().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(job) = guard.as_ref().filter(|job| job.id == request_id) {
        job.cancelled.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
pub async fn search_files(
    query: String,
    offset: u32,
    request_id: String,
) -> Result<SearchResult, String> {
    let request = Request {
        query: query.trim().to_owned(),
        offset,
        limit: PAGE_SIZE,
        force_legacy: false,
    };
    validate_request(&request)?;
    if request_id.len() > 128 {
        return Err("无效的请求标识".into());
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut guard = active().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = guard.replace(Active {
            id: request_id.clone(),
            cancelled: cancelled.clone(),
        }) {
            previous.cancelled.store(true, Ordering::Relaxed);
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        let result = query_worker(&request, &cancelled);
        let mut guard = active().lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_some_and(|job| job.id == request_id) {
            *guard = None;
            if let Ok(result) = &result {
                let mut cache = results().lock().unwrap_or_else(|e| e.into_inner());
                if offset == 0 || cache.len() >= 500 {
                    cache.clear();
                }
                cache.extend(
                    result
                        .items
                        .iter()
                        .map(|entry| (entry.id.clone(), entry.clone())),
                );
            }
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

fn query_worker(request: &Request, cancelled: &AtomicBool) -> Result<SearchResult, String> {
    use std::process::Command;
    let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command.arg(WORKER_ARG);
    run_worker_command(command, request, cancelled, Duration::from_secs(5))
}

fn run_worker_command(
    mut command: std::process::Command,
    request: &Request,
    cancelled: &AtomicBool,
    timeout: Duration,
) -> Result<SearchResult, String> {
    use std::process::Stdio;
    if cancelled.load(Ordering::Relaxed) {
        return Err("搜索已取消".into());
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动文件搜索：{e}"))?;
    let output = child.stdout.take().ok_or("搜索结果管道不可用")?;
    // Drain concurrently: long Unicode paths can exceed a Windows pipe's buffer.
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        output
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let input = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    let write_result = child
        .stdin
        .take()
        .ok_or("搜索请求管道不可用")
        .and_then(|mut pipe| pipe.write_all(&input).map_err(|_| "无法发送搜索请求"));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if write_result.is_err() || cancelled.load(Ordering::Relaxed) || Instant::now() >= deadline
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            if cancelled.load(Ordering::Relaxed) {
                return Err("搜索已取消".into());
            }
            return Ok(SearchResult::status(
                "error",
                "Everything 响应超时，请稍后重试或检查索引状态",
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(error.to_string());
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "读取搜索结果失败")?
        .map_err(|e| e.to_string())?;
    if !status.success() || bytes.len() as u64 > MAX_OUTPUT {
        return Err("文件搜索进程异常，请重试".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "无法解析文件搜索结果".into())
}

#[tauri::command]
pub async fn file_action(app: tauri::AppHandle, id: String, action: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    use tauri_plugin_opener::OpenerExt;
    let entry = results()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned()
        .ok_or("搜索结果已过期，请重新搜索")?;
    if action == "copy" {
        return app
            .clipboard()
            .write_text(entry.path)
            .map_err(|e| e.to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        if !std::path::Path::new(&entry.path).exists() {
            return Err("文件已移动、删除或所在磁盘已断开，请重新搜索".into());
        }
        match action.as_str() {
            "open" => crate::launch::shell_execute(&entry.path, "", None, false),
            "reveal" => app
                .opener()
                .reveal_item_in_dir(&entry.path)
                .map_err(|e| e.to_string()),
            _ => Err("不支持的文件操作".into()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn start_everything() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut candidates = Vec::new();
        #[cfg(windows)]
        for hive in [
            winreg::enums::HKEY_CURRENT_USER,
            winreg::enums::HKEY_LOCAL_MACHINE,
        ] {
            // Both installer scopes and registry views; installation need not be
            // in Program Files. Read locations only, never execute uninstall strings.
            for view in [
                winreg::enums::KEY_WOW64_64KEY,
                winreg::enums::KEY_WOW64_32KEY,
            ] {
                for (subkey, value, directory) in [
                    (
                        r"Software\Microsoft\Windows\CurrentVersion\App Paths\Everything.exe",
                        "",
                        false,
                    ),
                    (r"Software\voidtools\Everything", "InstallLocation", true),
                    (
                        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Everything",
                        "InstallLocation",
                        true,
                    ),
                ] {
                    if let Ok(key) = winreg::RegKey::predef(hive)
                        .open_subkey_with_flags(subkey, winreg::enums::KEY_READ | view)
                    {
                        if let Ok(value) = key.get_value::<String, _>(value) {
                            if let Some(path) = installation_candidate(&value, directory) {
                                candidates.push(path);
                            }
                        }
                    }
                }
            }
        }
        for variable in [
            "ProgramW6432",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "LOCALAPPDATA",
        ] {
            if let Some(root) = std::env::var_os(variable) {
                candidates.push(std::path::PathBuf::from(root).join("Everything/Everything.exe"));
            }
        }
        let path = candidates
            .into_iter()
            .find(|path| path.is_file())
            .ok_or("未找到 Everything，请先从 voidtools.com 安装普通版，或手动启动便携版")?;
        crate::launch::shell_execute(&path.to_string_lossy(), "-startup", None, false)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn installation_candidate(value: &str, directory: bool) -> Option<std::path::PathBuf> {
    let value = value.trim().trim_matches('"');
    if value.is_empty() || value.contains('\0') {
        return None;
    }
    let path = std::path::PathBuf::from(value);
    if !path.is_absolute() {
        return None;
    }
    Some(if directory {
        path.join("Everything.exe")
    } else {
        path
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn registered_custom_install_locations_support_spaces_and_unicode_without_using_commands() {
        assert_eq!(
            installation_candidate(r#""D:\工具 & Apps\Everything""#, true).unwrap(),
            std::path::PathBuf::from(r"D:\工具 & Apps\Everything\Everything.exe")
        );
        assert_eq!(
            installation_candidate(r#""D:\工具\Everything.exe""#, false).unwrap(),
            std::path::PathBuf::from(r"D:\工具\Everything.exe")
        );
        for value in ["", "relative/path", "\0"] {
            assert!(installation_candidate(value, true).is_none());
        }
    }
    #[test]
    fn slow_worker_fixture() {
        if std::env::var_os("LUMA_TEST_SLOW_SEARCH_WORKER").is_some() {
            std::thread::sleep(Duration::from_secs(30));
        }
    }
    fn slow_worker() -> std::process::Command {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "file_search::tests::slow_worker_fixture",
                "--nocapture",
            ])
            .env("LUMA_TEST_SLOW_SEARCH_WORKER", "1");
        command
    }
    #[test]
    fn hung_query_is_killed_reaped_and_its_output_pipe_closed_on_timeout() {
        let request = Request {
            query: "fixture".into(),
            offset: 0,
            limit: 50,
            force_legacy: false,
        };
        let start = Instant::now();
        let response = run_worker_command(
            slow_worker(),
            &request,
            &AtomicBool::new(false),
            Duration::from_millis(120),
        )
        .unwrap();
        assert_eq!(response.state, "error");
        assert!(response.message.contains("超时"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn cancelling_an_in_flight_query_reaps_worker_without_waiting_for_timeout() {
        let request = Request {
            query: "fixture".into(),
            offset: 0,
            limit: 50,
            force_legacy: false,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let cancel = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            signal.store(true, Ordering::Relaxed);
        });
        let start = Instant::now();
        let error = run_worker_command(slow_worker(), &request, &cancelled, Duration::from_secs(5))
            .unwrap_err();
        cancel.join().unwrap();
        assert_eq!(error, "搜索已取消");
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn request_limits_block_unbounded_queries_and_invalid_strings() {
        let mut request = Request {
            query: "中文 & work ext:pdf".into(),
            offset: 0,
            limit: 50,
            force_legacy: false,
        };
        assert!(validate_request(&request).is_ok());
        request.query.push('\0');
        assert!(validate_request(&request).is_err());
        request.query = "字".repeat(1025);
        assert!(validate_request(&request).is_err());
        request.query.clear();
        request.limit = 51;
        assert!(validate_request(&request).is_err());
        request.limit = 50;
        request.offset = 100001;
        assert!(validate_request(&request).is_err());
    }
    #[test]
    fn file_identity_preserves_unicode_and_distinguishes_same_named_files() {
        let entry = FileEntry::new(r"C:\测试 & work\报价 单.pdf".into(), false, Some(42));
        assert_eq!(entry.name, "报价 单.pdf");
        assert_ne!(
            entry.id,
            FileEntry::new(r"D:\报价 单.pdf".into(), false, None).id
        );
        assert_eq!(entry.id, FileEntry::new(entry.path.clone(), false, None).id);
    }
}
