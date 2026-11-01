//! 用户主动检查官方 Release；下载后按 GitHub 的资产摘要校验再交给 NSIS 升级。
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::Manager;
const API: &str = "https://api.github.com/repos/fengzhouxuan/Luma/releases/latest";
const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;
static INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    body: Option<String>,
    published_at: Option<String>,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub version: String,
    pub notes: String,
    pub published_at: String,
    pub available: bool,
    pub can_install: bool,
    pub message: String,
}
struct Candidate {
    version: String,
    url: String,
    size: u64,
    sha256: String,
}

fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent(concat!("Luma/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let allowed = [
                "github.com",
                "api.github.com",
                "release-assets.githubusercontent.com",
                "objects.githubusercontent.com",
                "releases.githubusercontent.com",
            ];
            if attempt.previous().len() < 5
                && attempt.url().scheme() == "https"
                && attempt
                    .url()
                    .host_str()
                    .is_some_and(|host| allowed.contains(&host))
            {
                attempt.follow()
            } else {
                attempt.error("更新下载重定向到非官方地址")
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

fn candidate(release: &Release, current: &str) -> Result<Option<Candidate>, String> {
    let version = semver::Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .map_err(|_| "Release 版本号无效")?;
    let current = semver::Version::parse(current).map_err(|_| "当前版本号无效")?;
    if release.draft || release.prerelease || !version.pre.is_empty() || version <= current {
        return Ok(None);
    }
    let name = format!("Luma_{version}_x64-setup.exe");
    let url = format!(
        "https://github.com/fengzhouxuan/Luma/releases/download/{}/{name}",
        release.tag_name
    );
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == name && asset.browser_download_url == url)
        .ok_or("新版本缺少官方 x64 EXE 安装包")?;
    let digest = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("安装包缺少有效 SHA-256 摘要，请到官方 Release 手动安装")?;
    if asset.size == 0 || asset.size > MAX_DOWNLOAD {
        return Err("安装包大小超出允许范围".into());
    }
    Ok(Some(Candidate {
        version: version.to_string(),
        url,
        size: asset.size,
        sha256: digest.to_ascii_lowercase(),
    }))
}

fn installed_directory() -> Result<PathBuf, String> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let location: String = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Luma")
        .and_then(|key| key.get_value("InstallLocation"))
        .map_err(|_| "仅当前用户的 EXE 安装版支持应用内升级；请从官方 Release 下载对应安装包")?;
    let location = PathBuf::from(location.trim_matches('"'));
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let installed = location.join("luma.exe");
    if !installed.is_file()
        || crate::model::identity(&installed.to_string_lossy(), "")
            != crate::model::identity(&exe.to_string_lossy(), "")
    {
        return Err("当前运行的不是已安装版本，请先退出开发版并启动安装版".into());
    }
    Ok(location)
}

fn latest(client: &Client) -> Result<Release, String> {
    let response = client
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|e| format!("检查更新失败，请稍后重试：{e}"))?;
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Release 信息过大".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Release 信息无效：{e}"))
}

#[tauri::command]
pub async fn check_update() -> Result<UpdateInfo, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let release = latest(&client()?)?;
        let result = candidate(&release, env!("CARGO_PKG_VERSION"));
        let directory = installed_directory();
        let available = semver::Version::parse(release.tag_name.trim_start_matches('v'))
            .ok()
            .is_some_and(|version| {
                version > semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
                    && !release.draft
                    && !release.prerelease
            });
        let (can_install, message) = match (&result, &directory) {
            (Err(error), _) | (_, Err(error)) => (false, error.clone()),
            (Ok(Some(_)), Ok(_)) => (
                true,
                "下载完成并通过大小和 SHA-256 校验后，将退出并升级 Luma。".into(),
            ),
            _ => (
                false,
                "当前已是最新版本，或当前源码版高于已发布版本。".into(),
            ),
        };
        Ok(UpdateInfo {
            current: env!("CARGO_PKG_VERSION").into(),
            version: release.tag_name,
            notes: release.body.unwrap_or_default(),
            published_at: release.published_at.unwrap_or_default(),
            available,
            can_install,
            message,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn verify(bytes: &[u8], expected: &Candidate) -> Result<(), String> {
    if bytes.len() as u64 != expected.size || bytes.len() as u64 > MAX_DOWNLOAD {
        return Err("安装包大小不匹配，已取消升级".into());
    }
    let digest = format!("{:x}", Sha256::digest(bytes));
    if digest != expected.sha256 {
        return Err("安装包 SHA-256 不匹配，已取消升级".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn install_update(app: tauri::AppHandle, version: String) -> Result<(), String> {
    if INSTALLING.swap(true, Ordering::AcqRel) {
        return Err("更新正在进行".into());
    }
    let result = tauri::async_runtime::spawn_blocking(move || {
        let directory = installed_directory()?;
        let client = client()?;
        let release = latest(&client)?;
        let expected =
            candidate(&release, env!("CARGO_PKG_VERSION"))?.ok_or("没有可安装的新版本")?;
        if version != release.tag_name {
            return Err("检查后 Release 已变化，请重新检查并确认更新日志".into());
        }
        let mut bytes = Vec::new();
        client
            .get(&expected.url)
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|e| e.to_string())?
            .take(MAX_DOWNLOAD + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        verify(&bytes, &expected)?;
        let cache = app
            .path()
            .app_cache_dir()
            .map_err(|e| e.to_string())?
            .join("updates");
        std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let installer = cache.join(format!("Luma-{}-{stamp}-setup.exe", expected.version));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&installer)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        // 再读落盘内容，防止不完整写入；Command 参数单独传递，不经过 shell。
        verify(
            &std::fs::read(&installer).map_err(|e| e.to_string())?,
            &expected,
        )?;
        let mut command = std::process::Command::new(&installer);
        command.args(["/P", "/UPDATE", "/R"]);
        // NSIS 要求 /D 为最后一个、不带引号的参数，即使目录含空格；不经过 cmd。
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.raw_arg(format!("/D={}", directory.display()));
        }
        command
            .spawn()
            .map_err(|e| format!("无法启动升级安装包：{e}"))?;
        app.exit(0);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|result| result);
    INSTALLING.store(false, Ordering::Release);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release() -> Release {
        Release {tag_name:"v9.0.0".into(),body:None,published_at:None,draft:false,prerelease:false,assets:vec![Asset{name:"Luma_9.0.0_x64-setup.exe".into(),browser_download_url:"https://github.com/fengzhouxuan/Luma/releases/download/v9.0.0/Luma_9.0.0_x64-setup.exe".into(),size:3,digest:Some(format!("sha256:{:x}",Sha256::digest(b"abc")))}]}
    }
    #[test]
    fn rejects_downgrades_untrusted_urls_and_missing_digests() {
        assert!(candidate(&release(), "10.0.0").unwrap().is_none());
        let mut value = release();
        value.assets[0].browser_download_url = "https://evil.example/setup.exe".into();
        assert!(candidate(&value, "1.0.0").is_err());
        let mut value = release();
        value.assets[0].digest = None;
        assert!(candidate(&value, "1.0.0").is_err());
        let mut value = release();
        value.prerelease = true;
        assert!(candidate(&value, "1.0.0").unwrap().is_none());
    }
    #[test]
    fn accepts_exact_payload_and_rejects_corrupted_or_truncated_installers() {
        let expected = candidate(&release(), "1.0.0").unwrap().unwrap();
        assert!(verify(b"abc", &expected).is_ok());
        assert!(verify(b"abd", &expected).is_err());
        assert!(verify(b"ab", &expected).is_err());
    }
}
