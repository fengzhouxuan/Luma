use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize, Clone, Debug)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub shortcut_path: Option<String>,
    pub arguments: String,
    pub working_dir: Option<String>,
    pub source: String,
    pub source_path: String,
    pub icon: Option<String>,
    pub hidden_reason: Option<String>,
    pub category: String,
}

pub fn identity(path: &str, arguments: &str) -> String {
    if path.contains("://") {
        return serde_json::to_string(&(path, arguments.trim())).expect("字符串可以序列化");
    }
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let path = path.to_string_lossy().replace('/', "\\").to_lowercase();
    serde_json::to_string(&(path, arguments.trim())).expect("字符串可以序列化")
}

impl AppEntry {
    pub fn new(name: String, path: String, source: &str, source_path: String) -> Self {
        Self {
            id: identity(&path, ""),
            hidden_reason: crate::classify_noise(&name, &path),
            category: crate::auto_category(&name, &path),
            name,
            path,
            source: source.into(),
            source_path,
            shortcut_path: None,
            arguments: String::new(),
            working_dir: None,
            icon: None,
        }
    }

    pub fn update_identity(&mut self) {
        self.id = identity(&self.path, &self.arguments);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinguishes_profiles_and_same_named_programs() {
        assert_ne!(
            identity("com.epicgames.launcher://apps/GameID?action=launch", ""),
            identity("com.epicgames.launcher://apps/gameid?action=launch", "")
        );
        assert_ne!(
            identity(r"C:\Apps\editor.exe", "--profile work"),
            identity(r"C:\Apps\editor.exe", "--profile home")
        );
        assert_ne!(
            identity(r"C:\Apps\editor.exe", ""),
            identity(r"D:\Apps\editor.exe", "")
        );
        assert_eq!(
            identity(r"C:\Apps\Editor.exe", "  --profile Work  "),
            identity(r"c:\apps\editor.exe", "--profile Work")
        );
        assert_ne!(
            identity(r"C:\Apps\editor.exe", "--profile Work"),
            identity(r"C:\Apps\editor.exe", "--profile work")
        );
    }
}
