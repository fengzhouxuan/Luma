//! 使用系统已知文件夹定位快捷方式，兼容桌面重定向与 ClickOnce。
use crate::model::AppEntry;
use std::{collections::HashSet, path::PathBuf};

pub fn roots() -> Vec<(PathBuf, &'static str)> {
    #[cfg(windows)]
    {
        use windows::Win32::{
            System::Com::CoTaskMemFree,
            UI::Shell::{
                FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_Programs,
                FOLDERID_PublicDesktop, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
            },
        };
        let mut roots = Vec::new();
        let mut seen = HashSet::new();
        for (folder, label) in [
            (FOLDERID_Programs, "开始菜单"),
            (FOLDERID_CommonPrograms, "公共开始菜单"),
            (FOLDERID_Desktop, "桌面"),
            (FOLDERID_PublicDesktop, "公共桌面"),
        ] {
            unsafe {
                match SHGetKnownFolderPath(&folder, KF_FLAG_DEFAULT, None) {
                    Ok(value) => {
                        let path = value.to_string().ok().map(PathBuf::from);
                        CoTaskMemFree(Some(value.0.cast()));
                        if let Some(path) = path {
                            if seen.insert(path.to_string_lossy().to_lowercase()) {
                                roots.push((path, label));
                            }
                        }
                    }
                    Err(error) => {
                        crate::diagnostics::record("扫描", label, None, error.to_string())
                    }
                }
            }
        }
        roots
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn game_url(bytes: &[u8]) -> Option<String> {
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        String::from_utf16(
            &bytes[2..]
                .chunks_exact(2)
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .collect::<Vec<_>>(),
        )
        .ok()?
    } else {
        String::from_utf8_lossy(bytes)
            .trim_start_matches('\u{feff}')
            .to_string()
    };
    let mut internet_shortcut = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            internet_shortcut = line.eq_ignore_ascii_case("[InternetShortcut]");
            continue;
        }
        if !internet_shortcut {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim();
            if key.trim().eq_ignore_ascii_case("URL") {
                let scheme = value.split_once("://")?.0.to_ascii_lowercase();
                if [
                    "steam",
                    "com.epicgames.launcher",
                    "goggalaxy",
                    "uplay",
                    "battlenet",
                    "origin",
                    "ea",
                    "itch",
                ]
                .contains(&scheme.as_str())
                    && !value.chars().any(char::is_control)
                {
                    return Some(value.into());
                }
            }
        }
    }
    None
}

fn parse(path: &PathBuf, source: &str) -> Option<AppEntry> {
    let ext = path.extension()?.to_str()?;
    let mut app = if ext.eq_ignore_ascii_case("lnk") {
        crate::parse_lnk(path)?
    } else {
        let name = path.file_stem()?.to_string_lossy().to_string();
        let target = if ext.eq_ignore_ascii_case("appref-ms") {
            path.to_string_lossy().to_string()
        } else if ext.eq_ignore_ascii_case("url") {
            game_url(&std::fs::read(path).ok()?)?
        } else {
            return None;
        };
        let mut app = AppEntry::new(name, target, source, path.to_string_lossy().to_string());
        app.shortcut_path = Some(path.to_string_lossy().to_string());
        if ext.eq_ignore_ascii_case("url") {
            app.category = "游戏".into();
        }
        app
    };
    app.source = source.into();
    Some(app)
}

pub fn scan(apps: &mut Vec<AppEntry>, seen: &mut HashSet<String>) {
    for (directory, source) in roots() {
        for extension in ["lnk", "appref-ms", "url"] {
            let pattern = directory
                .join(format!("**/*.{extension}"))
                .to_string_lossy()
                .to_string();
            let options = glob::MatchOptions {
                case_sensitive: false,
                ..Default::default()
            };
            if let Ok(paths) = glob::glob_with(&pattern, options) {
                for path in paths.flatten() {
                    if let Some(app) = parse(&path, source) {
                        if seen.insert(app.id.clone()) {
                            apps.push(app);
                        }
                    } else if extension != "url" {
                        crate::diagnostics::record(
                            "扫描",
                            &path.to_string_lossy(),
                            None,
                            "快捷方式无法解析或目标不存在",
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_game_protocols_without_importing_web_bookmarks() {
        assert_eq!(
            game_url(b"[InternetShortcut]\r\nURL=steam://rungameid/123\r\n").as_deref(),
            Some("steam://rungameid/123")
        );
        assert!(game_url(b"[InternetShortcut]\nURL=https://example.com/").is_none());
        assert!(game_url(b"[Other]\nURL=steam://rungameid/123").is_none());
        let mut unicode = vec![0xff, 0xfe];
        unicode.extend(
            "[InternetShortcut]\nURL=com.epicgames.launcher://apps/GameID?action=launch"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        assert!(game_url(&unicode).unwrap().contains("GameID"));
    }
    #[test]
    fn clickonce_reference_is_preserved_without_executing_it() {
        let path = PathBuf::from(r"C:\Users\Example\Desktop\Profile.appref-ms");
        let app = parse(&path, "桌面").unwrap();
        assert_eq!(app.path, path.to_string_lossy());
        assert_eq!(app.shortcut_path.as_deref(), Some(app.path.as_str()));
        assert_eq!(app.source, "桌面");
    }
}
