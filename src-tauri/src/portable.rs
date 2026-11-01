//! 仅扫描明确的用户程序和包管理器入口，不遍历 PATH 或整盘。
use crate::model::AppEntry;
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub fn roots() -> Vec<(PathBuf, &'static str)> {
    let mut roots = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push((PathBuf::from(&local).join("Programs"), "用户程序目录"));
        roots.push((
            PathBuf::from(local).join("Microsoft/WinGet/Links"),
            "WinGet 便携应用",
        ));
    }
    if let Some(programs) = std::env::var_os("ProgramFiles") {
        roots.push((
            PathBuf::from(programs).join("WinGet/Links"),
            "WinGet 便携应用",
        ));
    }
    for (variable, fallback) in [
        (
            "SCOOP",
            std::env::var_os("USERPROFILE").map(|path| PathBuf::from(path).join("scoop")),
        ),
        (
            "SCOOP_GLOBAL",
            std::env::var_os("ProgramData").map(|path| PathBuf::from(path).join("scoop")),
        ),
    ] {
        if let Some(root) = std::env::var_os(variable).map(PathBuf::from).or(fallback) {
            roots.push((root.join("apps"), "Scoop 便携应用"));
        }
    }
    roots
}

pub(crate) fn gui_executable(path: &Path) -> bool {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return false;
    }
    let name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    if crate::is_noise_exe(&name)
        || [
            "elevate",
            "chrome_proxy",
            "squirrel",
            "qtwebengineprocess",
            "cefsharp.browsersubprocess",
        ]
        .contains(&name.as_str())
    {
        return false;
    }
    let read = || -> Option<bool> {
        let mut file = File::open(path).ok()?;
        let mut magic = [0u8; 2];
        file.read_exact(&mut magic).ok()?;
        if &magic != b"MZ" {
            return None;
        }
        file.seek(SeekFrom::Start(0x3c)).ok()?;
        let mut offset = [0u8; 4];
        file.read_exact(&mut offset).ok()?;
        let offset = u32::from_le_bytes(offset) as u64;
        if offset > 16 * 1024 * 1024 {
            return None;
        }
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut signature = [0u8; 4];
        file.read_exact(&mut signature).ok()?;
        if &signature != b"PE\0\0" {
            return None;
        }
        file.seek(SeekFrom::Start(offset + 24)).ok()?;
        file.read_exact(&mut magic).ok()?;
        if ![0x10b, 0x20b].contains(&u16::from_le_bytes(magic)) {
            return None;
        }
        file.seek(SeekFrom::Start(offset + 24 + 68)).ok()?;
        file.read_exact(&mut magic).ok()?;
        Some(u16::from_le_bytes(magic) == 2)
    };
    read() == Some(true)
}

fn scan_directory(
    directory: &Path,
    source: &str,
    depth: u8,
    apps: &mut Vec<AppEntry>,
    seen: &mut HashSet<String>,
) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && gui_executable(&path) {
            let target = std::fs::canonicalize(&path).unwrap_or(path);
            let path = target.to_string_lossy().to_string();
            let name = crate::friendly_name(&path).unwrap_or_else(|| {
                target
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            let app = AppEntry::new(name, path, source, directory.to_string_lossy().to_string());
            if seen.insert(app.id.clone()) {
                apps.push(app);
            }
        } else if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if ["bin", "app", "application", "current"].contains(&name.as_str()) {
                scan_directory(&path, source, depth - 1, apps, seen);
            }
        }
    }
}

fn scoop_shortcuts(
    directory: &Path,
    manifest: &serde_json::Value,
    architecture: &str,
) -> Vec<AppEntry> {
    let shortcuts = manifest
        .get("architecture")
        .and_then(|value| value.get(architecture))
        .and_then(|value| value.get("shortcuts"))
        .or_else(|| manifest.get("shortcuts"))
        .and_then(serde_json::Value::as_array);
    let mut apps = Vec::new();
    for shortcut in shortcuts.into_iter().flatten() {
        let Some(values) = shortcut.as_array() else {
            continue;
        };
        let (Some(target), Some(name)) = (
            values.first().and_then(serde_json::Value::as_str),
            values.get(1).and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        let target = directory.join(target.replace("$dir", &directory.to_string_lossy()));
        if !gui_executable(&target) {
            continue;
        }
        let mut app = AppEntry::new(
            name.into(),
            target.to_string_lossy().to_string(),
            "Scoop 便携应用",
            directory.to_string_lossy().to_string(),
        );
        app.arguments = values
            .get(2)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .replace("$dir", &directory.to_string_lossy());
        app.working_dir = Some(directory.to_string_lossy().to_string());
        app.update_identity();
        apps.push(app);
    }
    apps
}

fn scan_scoop(directory: &Path, apps: &mut Vec<AppEntry>, seen: &mut HashSet<String>) {
    let read_json = |names: &[&str]| {
        names.iter().find_map(|name| {
            std::fs::read(directory.join(name))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        })
    };
    let metadata = read_json(&["scoop-install.json", "install.json"]);
    let default_architecture = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_pointer_width = "64") {
        "64bit"
    } else {
        "32bit"
    };
    let architecture = metadata
        .as_ref()
        .and_then(|value| value.get("architecture"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(default_architecture);
    if let Some(manifest) = read_json(&["scoop-manifest.json", "manifest.json"]) {
        let shortcuts = scoop_shortcuts(directory, &manifest, architecture);
        let has_shortcuts = !shortcuts.is_empty();
        for app in shortcuts {
            if seen.insert(app.id.clone()) {
                apps.push(app);
            }
        }
        if has_shortcuts {
            return;
        }
    }
    // 仅补充根部 GUI EXE；不把 Java/Qt 等包的 bin 目录运行时全部列出来。
    scan_directory(directory, "Scoop 便携应用", 1, apps, seen);
}

pub fn scan(apps: &mut Vec<AppEntry>, seen: &mut HashSet<String>) {
    for (root, source) in roots() {
        if source == "用户程序目录" {
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            scan_directory(&root, source, 1, apps, seen);
            for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
                if !entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("Common")
                {
                    scan_directory(&entry.path(), source, 3, apps, seen);
                }
            }
        } else if source == "Scoop 便携应用" {
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    scan_scoop(&entry.path().join("current"), apps, seen);
                }
            }
        } else {
            scan_directory(&root, source, 1, apps, seen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gui_detection_uses_pe_subsystem_and_excludes_command_line_tools() {
        let root = std::env::temp_dir().join(format!("luma-pe-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("probe.exe");
        let mut bytes = vec![0u8; 256];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        bytes[156..158].copy_from_slice(&2u16.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(gui_executable(&path));
        let manifest = serde_json::json!({"shortcuts": [["probe.exe", "Probe", "--profile Work"]]});
        let apps = scoop_shortcuts(&root, &manifest, "64bit");
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].arguments, "--profile Work");
        assert_eq!(apps[0].working_dir.as_deref(), root.to_str());
        let architecture_manifest = serde_json::json!({"shortcuts": [["probe.exe", "Default"]], "architecture": {"64bit": {"shortcuts": [["probe.exe", "Architecture override", "--arch=64"]]}}});
        let apps = scoop_shortcuts(&root, &architecture_manifest, "64bit");
        assert_eq!(apps[0].arguments, "--arch=64");
        bytes[156..158].copy_from_slice(&3u16.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(!gui_executable(&path));
        std::fs::write(&path, b"not an executable").unwrap();
        assert!(!gui_executable(&path));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
