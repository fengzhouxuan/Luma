//! Browser discovery identities ignore shortcut attribution, never launch modes.
use std::{borrow::Cow, path::Path};

pub fn is_browser(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        name.eq_ignore_ascii_case("chrome.exe") || name.eq_ignore_ascii_case("msedge.exe")
    })
}

pub fn identity_arguments<'a>(path: &Path, arguments: &'a str) -> Cow<'a, str> {
    let arguments = arguments.trim();
    if !is_browser(path) || !arguments.contains("--source-shortcut-location=") {
        return Cow::Borrowed(arguments);
    }
    // Preserve raw tokens, quoted payloads and escape sequences. Never split a
    // URL/profile value at spaces or remove a switch after the `--` delimiter.
    let mut tokens = Vec::new();
    let mut start = None;
    let mut quoted = false;
    let mut slashes = 0;
    for (i, byte) in arguments.bytes().enumerate() {
        if start.is_none() && matches!(byte, b' ' | b'\t') {
            continue;
        }
        start.get_or_insert(i);
        if byte == b'"' && slashes % 2 == 0 {
            quoted = !quoted;
        }
        if matches!(byte, b' ' | b'\t') && !quoted {
            tokens.push(&arguments[start.take().unwrap()..i]);
        }
        slashes = if byte == b'\\' { slashes + 1 } else { 0 };
    }
    if quoted {
        return Cow::Borrowed(arguments);
    }
    if let Some(start) = start {
        tokens.push(&arguments[start..]);
    }
    let mut switches = true;
    let mut removed = false;
    tokens.retain(|raw| {
        let token = raw
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(raw);
        if token == "--" {
            switches = false;
        }
        let attribution = switches
            && token
                .strip_prefix("--source-shortcut-location=")
                .is_some_and(|value| {
                    !value.is_empty()
                        && value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                });
        removed |= attribution;
        !attribution
    });
    if removed {
        Cow::Owned(tokens.join(" "))
    } else {
        Cow::Borrowed(arguments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_identity::version_target as stable_target;
    use std::path::PathBuf;
    #[test]
    fn attribution_is_ignored_but_profiles_modes_and_payloads_remain_distinct() {
        let chrome = Path::new(r"C:\Chrome\chrome.exe");
        assert_eq!(
            identity_arguments(chrome, "--source-shortcut-location=start-menu"),
            ""
        );
        assert_eq!(
            identity_arguments(
                chrome,
                r#"--profile-directory="Profile 1" --source-shortcut-location=desktop --incognito"#
            ),
            r#"--profile-directory="Profile 1" --incognito"#
        );
        assert_eq!(
            identity_arguments(chrome, r#""--source-shortcut-location=start-menu""#),
            ""
        );
        for args in [
            r#"--app="https://example.test/a --source-shortcut-location=desktop""#,
            "-- --source-shortcut-location=desktop",
            "--source-shortcut-location=",
            r#"--profile-directory="unterminated --source-shortcut-location=desktop"#,
        ] {
            assert_eq!(identity_arguments(chrome, args), args);
        }
        assert_eq!(
            identity_arguments(
                Path::new(r"C:\Other\editor.exe"),
                "--source-shortcut-location=desktop"
            ),
            "--source-shortcut-location=desktop"
        );
        let id = |args| crate::model::identity(&chrome.to_string_lossy(), args);
        assert_eq!(id(""), id("--source-shortcut-location=start-menu"));
        for args in [
            "--incognito",
            "--inprivate",
            "--profile-directory=Work",
            "--app-id=abc",
            "https://example.test",
        ] {
            assert_ne!(id(""), id(args));
        }
        assert_ne!(
            id("--profile-directory=Work"),
            id("--profile-directory=Home")
        );
    }

    #[test]
    fn only_identical_version_copies_with_a_stable_launcher_are_folded() {
        let root = std::env::temp_dir().join(format!(
            "luma-browser-{}-{}",
            std::process::id(),
            crate::diagnostics::now()
        ));
        let application = root.join("Application");
        let version = application.join("156.0.1.2");
        std::fs::create_dir_all(&version).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for file in ["Application/chrome.exe", "Application/156.0.1.2/chrome.exe"] {
                    let _ = std::fs::remove_file(self.0.join(file));
                }
                for dir in ["Application/156.0.1.2", "Application", ""] {
                    let _ = std::fs::remove_dir(self.0.join(dir));
                }
            }
        }
        let _cleanup = Cleanup(root);
        let copy = version.join("chrome.exe");
        let stable = application.join("chrome.exe");
        std::fs::write(&copy, b"browser v1").unwrap();
        assert_eq!(stable_target(&copy), None);
        std::fs::write(&stable, b"browser v1").unwrap();
        assert_eq!(stable_target(&copy), Some(stable.clone()));
        assert_eq!(
            crate::model::identity(&copy.to_string_lossy(), ""),
            crate::model::identity(&stable.to_string_lossy(), "")
        );
        assert_eq!(
            crate::discovery::installation_target(
                "Google Chrome",
                &application.to_string_lossy(),
                &copy.to_string_lossy()
            ),
            Some(stable.clone())
        );
        std::fs::write(&stable, b"browser v2").unwrap();
        assert_eq!(stable_target(&copy), None);
        assert_ne!(
            crate::model::identity(&copy.to_string_lossy(), ""),
            crate::model::identity(&stable.to_string_lossy(), "")
        );
    }
}
