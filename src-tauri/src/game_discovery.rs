//! Unreal bootstrap resources identify the actual child process without running it.
use std::path::{Component, Path, PathBuf};

fn relative_target(bytes: &[u8]) -> Option<PathBuf> {
    if bytes.is_empty() || bytes.len() % 2 != 0 || bytes.len() > 4096 {
        return None;
    }
    let units: Vec<_> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    let text = String::from_utf16(&units).ok()?;
    let path = PathBuf::from(text);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    let parts: Vec<_> = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    if parts.len() < 4
        || parts[parts.len() - 3] != "binaries"
        || !["win64", "win32"].contains(&parts[parts.len() - 2].as_str())
        || !parts.last()?.ends_with("-shipping.exe")
    {
        return None;
    }
    Some(path)
}

#[cfg(windows)]
fn bootstrap_target(path: &Path) -> Option<PathBuf> {
    use windows_sys::Win32::{
        Foundation::FreeLibrary,
        System::LibraryLoader::{
            FindResourceW, LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE,
            LoadLibraryExW, LoadResource, LockResource, SizeofResource,
        },
    };
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > 2 * 1024 * 1024 || !crate::portable::gui_executable(path) {
        return None;
    }
    let wide: Vec<_> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let module = LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
        );
        if module.is_null() {
            return None;
        }
        struct Library(windows_sys::Win32::Foundation::HMODULE);
        impl Drop for Library {
            fn drop(&mut self) {
                unsafe {
                    FreeLibrary(self.0);
                }
            }
        }
        let _library = Library(module);
        let resource = FindResourceW(module, 201usize as *const u16, 10usize as *const u16);
        if resource.is_null() {
            return None;
        }
        let length = SizeofResource(module, resource) as usize;
        if length == 0 || length > 4096 {
            return None;
        }
        let handle = LoadResource(module, resource);
        if handle.is_null() {
            return None;
        }
        let data = LockResource(handle);
        if data.is_null() {
            return None;
        }
        let relative = relative_target(std::slice::from_raw_parts(data as *const u8, length))?;
        let parent = std::fs::canonicalize(path.parent()?).ok()?;
        let target = std::fs::canonicalize(parent.join(relative)).ok()?;
        if !target.starts_with(&parent) || !crate::portable::gui_executable(&target) {
            return None;
        }
        Some(target)
    }
}

#[cfg(not(windows))]
fn bootstrap_target(_path: &Path) -> Option<PathBuf> {
    None
}

pub fn annotate(apps: &mut [crate::model::AppEntry], cfg: &crate::config::Config) {
    let mut children = Vec::new();
    for app in apps
        .iter_mut()
        .filter(|app| app.source == "目录扫描" && app.arguments.is_empty())
    {
        if let Some(target) = bootstrap_target(Path::new(&app.path)) {
            children.push(crate::model::identity(&target.to_string_lossy(), ""));
            if app.name.eq_ignore_ascii_case("BootstrapPackagedGame") {
                if let Some(name) = Path::new(&app.path).file_stem() {
                    app.name = name.to_string_lossy().into_owned();
                }
            }
        }
    }
    for app in apps.iter_mut().filter(|app| children.contains(&app.id)) {
        if !cfg.pinned_apps.contains(&app.id)
            && !cfg.launch_overrides.contains_key(&app.id)
            && !cfg.aliases.contains_key(&app.id)
        {
            app.hidden_reason = Some("游戏引导器已提供入口，该程序为其实际游戏进程".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bootstrap_resources_must_be_bounded_relative_shipping_paths() {
        let bytes = |text: &str| {
            text.encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            relative_target(&bytes(r"Project\Binaries\Win64\Project-Win64-Shipping.exe")),
            Some(PathBuf::from(
                r"Project\Binaries\Win64\Project-Win64-Shipping.exe"
            ))
        );
        for value in [
            r"..\Project\Binaries\Win64\Project-Shipping.exe",
            r"C:\Project\Binaries\Win64\Project-Shipping.exe",
            r"Project\Binaries\Win64\helper.exe",
            r"https://example.test/Game-Shipping.exe",
        ] {
            assert!(relative_target(&bytes(value)).is_none());
        }
        assert!(relative_target(&[1]).is_none());
        assert!(relative_target(&vec![0; 4098]).is_none());
    }
}
