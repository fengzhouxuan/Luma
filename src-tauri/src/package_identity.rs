//! Resolve installed-package EXEs through Windows' manifest APIs, never names.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};
use windows::{
    Win32::{
        Storage::Packaging::Appx::{AppxFactory, GetPackagesByPackageFamily, IAppxFactory},
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, STGM_READ, STGM_SHARE_DENY_WRITE},
        UI::Shell::SHCreateStreamOnFileEx,
    },
    core::{PCWSTR, PWSTR, w},
};

#[derive(Clone, Debug)]
struct Manifest {
    family: String,
    applications: Vec<(String, String)>,
}
type Cache = HashMap<PathBuf, (SystemTime, u64, Manifest)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

pub fn root(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let text = path.to_string_lossy();
    if text.starts_with(r"\\") && !text.starts_with(r"\\?\") {
        return None;
    }
    path.ancestors()
        .find(|parent| {
            parent
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name.eq_ignore_ascii_case("WindowsApps"))
        })
        .map(Path::to_path_buf)
}

fn read(root: &Path) -> Option<Manifest> {
    let manifest = root.join("AppxManifest.xml");
    let metadata = std::fs::metadata(&manifest).ok()?;
    if metadata.len() > 4 * 1024 * 1024 {
        return None;
    }
    let stamp = metadata.modified().ok()?;
    let cache = CACHE.get_or_init(Default::default);
    if let Some((modified, length, value)) = cache.lock().ok()?.get(root) {
        if *modified == stamp && *length == metadata.len() {
            return Some(value.clone());
        }
    }
    let _apartment = crate::packaged::Apartment::enter().ok()?;
    let wide: Vec<u16> = manifest
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let value = unsafe {
        let factory: IAppxFactory =
            CoCreateInstance(&AppxFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = SHCreateStreamOnFileEx(
            PCWSTR(wide.as_ptr()),
            (STGM_READ | STGM_SHARE_DENY_WRITE).0,
            0,
            false,
            None,
        )
        .ok()?;
        let reader = factory.CreateManifestReader(&stream).ok()?;
        let family =
            crate::packaged::ShellString(reader.GetPackageId().ok()?.GetPackageFamilyName().ok()?)
                .text()
                .ok()?;
        let items = reader.GetApplications().ok()?;
        let mut applications = Vec::new();
        while items.GetHasCurrent().ok()?.as_bool() {
            if applications.len() >= 1024 {
                return None;
            }
            let item = items.GetCurrent().ok()?;
            if let (Ok(id), Ok(exe)) = (
                item.GetAppUserModelId(),
                item.GetStringValue(w!("Executable")),
            ) {
                let id = crate::packaged::ShellString(id).text().ok()?;
                let exe = crate::packaged::ShellString(exe).text().ok()?;
                applications.push((id, exe));
            }
            let _ = items.MoveNext().ok()?;
        }
        Manifest {
            family,
            applications,
        }
    };
    let mut cache = cache.lock().ok()?;
    if cache.len() >= 128 {
        cache.clear();
    }
    cache.insert(root.to_path_buf(), (stamp, metadata.len(), value.clone()));
    Some(value)
}

fn registered(root: &Path, family: &str) -> bool {
    let wide: Vec<u16> = family.encode_utf16().chain(Some(0)).collect();
    let mut count = 0;
    let mut length = 0;
    unsafe {
        let status =
            GetPackagesByPackageFamily(PCWSTR(wide.as_ptr()), &mut count, None, &mut length, None);
        if status.0 != 122 || count == 0 || count > 128 || length > 1024 * 1024 {
            return false;
        }
        let mut names = vec![PWSTR::null(); count as usize];
        let mut buffer = vec![0u16; length as usize];
        if GetPackagesByPackageFamily(
            PCWSTR(wide.as_ptr()),
            &mut count,
            Some(names.as_mut_ptr()),
            &mut length,
            Some(PWSTR(buffer.as_mut_ptr())),
        )
        .0 != 0
        {
            return false;
        }
        names.iter().take(count as usize).any(|name| {
            name.to_string().ok().is_some_and(|name| {
                root.file_name()
                    .is_some_and(|folder| folder.eq_ignore_ascii_case(&name))
            })
        })
    }
}

fn matches(root: &Path, path: &Path, manifest: &Manifest) -> Vec<String> {
    let canonical = std::fs::canonicalize(path).ok();
    let Some(canonical) = canonical else {
        return Vec::new();
    };
    manifest
        .applications
        .iter()
        .filter_map(|(id, exe)| {
            let relative = Path::new(exe);
            if relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return None;
            }
            let executable = std::fs::canonicalize(root.join(relative)).ok()?;
            executable
                .to_string_lossy()
                .eq_ignore_ascii_case(&canonical.to_string_lossy())
                .then(|| id.clone())
        })
        .collect()
}

pub fn canonical_target(path: &Path, arguments: &str) -> Option<String> {
    if !arguments.trim().is_empty() {
        return None;
    }
    let root = root(path)?;
    let manifest = read(&root)?;
    if !registered(&root, &manifest.family) {
        return None;
    }
    let ids = matches(&root, path, &manifest);
    (ids.len() == 1).then(|| format!("shell:AppsFolder\\{}", ids[0]))
}

pub fn internal_executable(path: &Path) -> bool {
    let Some(root) = root(path) else {
        return false;
    };
    let Some(manifest) = read(&root) else {
        return false;
    };
    registered(&root, &manifest.family)
        && !manifest.applications.is_empty()
        && matches(&root, path, &manifest).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_roots_are_explicit_and_manifest_targets_must_match_exactly() {
        assert_eq!(
            root(Path::new(
                r"C:\Program Files\WindowsApps\Family_1.0_x64__pub\app.exe"
            )),
            Some(PathBuf::from(
                r"C:\Program Files\WindowsApps\Family_1.0_x64__pub"
            ))
        );
        assert!(root(Path::new(r"C:\Other\Family\app.exe")).is_none());
        assert!(root(Path::new(r"\\server\WindowsApps\Family\app.exe")).is_none());
        let root = std::env::temp_dir().join(format!("luma-manifest-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0.join("app.exe"));
                let _ = std::fs::remove_file(self.0.join("server.exe"));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let target = root.join("app.exe");
        std::fs::write(&target, b"app").unwrap();
        let server = root.join("server.exe");
        std::fs::write(&server, b"server").unwrap();
        let manifest = Manifest {
            family: "Family_pub".into(),
            applications: vec![
                ("Family_pub!App".into(), "app.exe".into()),
                ("Family_pub!Other".into(), r"..\app.exe".into()),
            ],
        };
        assert_eq!(matches(&root, &target, &manifest), vec!["Family_pub!App"]);
        assert!(matches(&root, &server, &manifest).is_empty());
        let mut ambiguous = manifest;
        ambiguous
            .applications
            .push(("Family_pub!Second".into(), "app.exe".into()));
        assert_eq!(matches(&root, &target, &ambiguous).len(), 2);
        assert!(canonical_target(&target, "--profile work").is_none());
    }

    #[test]
    #[ignore = "只读本机包清单；使用 LUMA_PACKAGE_EXE / LUMA_EXPECT_AUMID 指定已安装应用"]
    fn host_manifest_identity() {
        let path = std::env::var("LUMA_PACKAGE_EXE").expect("需要指定 EXE");
        let expected = std::env::var("LUMA_EXPECT_AUMID").expect("需要指定 AUMID");
        assert_eq!(
            canonical_target(Path::new(&path), ""),
            Some(format!("shell:AppsFolder\\{expected}"))
        );
        assert_eq!(
            crate::model::identity(&path, ""),
            crate::model::identity(&format!("shell:AppsFolder\\{expected}"), "")
        );
        assert!(!internal_executable(Path::new(&path)));
        assert!(canonical_target(Path::new(&path), "--profile work").is_none());
    }
}
