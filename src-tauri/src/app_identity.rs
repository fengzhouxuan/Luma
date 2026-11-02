//! Known multi-executable launchers, rather than merging arbitrary equal names.
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

// Only fold an installation's version-directory copy when its bytes match the
// stable launcher. Other installations/channels/versions remain distinct.
pub fn version_target(path: &Path) -> Option<PathBuf> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    let version = path.parent()?;
    let name = version.file_name()?.to_str()?;
    let parts: Vec<_> = name.split('.').collect();
    if parts.len() != 4
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let application = version.parent()?;
    if !application.file_name()?.eq_ignore_ascii_case("Application") {
        return None;
    }
    let stable = application.join(path.file_name()?);
    let original = std::fs::metadata(path).ok()?;
    let candidate = std::fs::metadata(&stable).ok()?;
    if !original.is_file()
        || !candidate.is_file()
        || original.len() != candidate.len()
        || original.len() > 64 * 1024 * 1024
    {
        return None;
    }
    fn digest(path: &Path) -> Option<Vec<u8>> {
        let mut file = std::fs::File::open(path).ok()?;
        let mut hash = Sha256::new();
        let mut bytes = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut bytes).ok()?;
            if read == 0 {
                break;
            }
            hash.update(&bytes[..read]);
        }
        Some(hash.finalize().to_vec())
    }
    (digest(path)? == digest(&stable)?).then_some(stable)
}

pub fn ghub_target(path: &Path, arguments: &str) -> Option<PathBuf> {
    if !arguments.trim().is_empty()
        || !path.file_name()?.eq_ignore_ascii_case("lghub.exe")
        || !path.parent()?.file_name()?.eq_ignore_ascii_case("LGHUB")
    {
        return None;
    }
    // G HUB's official shortcut opens its tray launcher; DisplayIcon names the
    // UI executable. Require both components in this specific install layout.
    let launcher = path.parent()?.join("system_tray/lghub_system_tray.exe");
    (path.is_file() && launcher.is_file()).then_some(launcher)
}

pub fn migrated_id(id: &str) -> String {
    let Ok((path, arguments)) = serde_json::from_str::<(String, String)>(id) else {
        return id.into();
    };
    let target = Path::new(&path);
    #[cfg(windows)]
    let package = crate::package_identity::root(target).is_some();
    #[cfg(not(windows))]
    let package = false;
    if package
        || crate::browser::is_browser(target)
        || crate::app_identity::version_target(target).is_some()
        || ghub_target(target, &arguments).is_some()
    {
        crate::model::identity(&path, &arguments)
    } else {
        id.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghub_default_entries_merge_but_other_installs_arguments_and_missing_launchers_do_not() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("luma-ghub-{}-{nonce}", std::process::id()));
        let product = root.join("LGHUB");
        let tray = product.join("system_tray/lghub_system_tray.exe");
        let ui = product.join("lghub.exe");
        std::fs::create_dir_all(tray.parent().unwrap()).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for file in ["LGHUB/lghub.exe", "LGHUB/system_tray/lghub_system_tray.exe"] {
                    let _ = std::fs::remove_file(self.0.join(file));
                }
                for dir in ["LGHUB/system_tray", "LGHUB", ""] {
                    let _ = std::fs::remove_dir(self.0.join(dir));
                }
            }
        }
        let _cleanup = Cleanup(root.clone());
        std::fs::write(&ui, b"UI").unwrap();
        assert_eq!(ghub_target(&ui, ""), None);
        std::fs::write(&tray, b"tray launcher").unwrap();
        assert_eq!(ghub_target(&ui, ""), Some(tray.clone()));
        let identity = crate::model::identity;
        assert_eq!(
            identity(&ui.to_string_lossy(), ""),
            identity(&tray.to_string_lossy(), "")
        );
        assert_ne!(
            identity(&ui.to_string_lossy(), "--profile work"),
            identity(&tray.to_string_lossy(), "")
        );
        assert_ne!(
            identity(&root.join("Other/LGHUB/lghub.exe").to_string_lossy(), ""),
            identity(&tray.to_string_lossy(), "")
        );
        assert_eq!(ghub_target(&root.join("Other/lghub.exe"), ""), None);
        let canonical_ui = std::fs::canonicalize(&ui)
            .unwrap()
            .to_string_lossy()
            .to_lowercase();
        let old = serde_json::to_string(&(canonical_ui, "")).unwrap();
        let expected = identity(&tray.to_string_lossy(), "");
        assert_eq!(migrated_id(&old), expected);
        let mut cfg = crate::config::Config::default();
        cfg.pinned_apps = vec![old.clone(), expected.clone()];
        cfg.usage.insert(
            old,
            crate::config::Usage {
                count: 2,
                last_launch: 20,
            },
        );
        cfg.usage.insert(
            expected.clone(),
            crate::config::Usage {
                count: 3,
                last_launch: 10,
            },
        );
        crate::config::migrate_discovery_ids(&mut cfg);
        assert_eq!(cfg.pinned_apps, vec![expected.clone()]);
        assert_eq!(cfg.usage[&expected].count, 5);
        assert_eq!(cfg.usage[&expected].last_launch, 20);
        let shortcut = crate::model::AppEntry::new(
            "Logitech G HUB".into(),
            tray.to_string_lossy().into_owned(),
            "公共开始菜单",
            String::new(),
        );
        let installed = crate::model::AppEntry::new(
            "Logitech G HUB".into(),
            ui.to_string_lossy().into_owned(),
            "安装记录",
            String::new(),
        );
        assert_eq!(shortcut.id, installed.id);
        assert_eq!(installed.path, ui.to_string_lossy());
    }
}
