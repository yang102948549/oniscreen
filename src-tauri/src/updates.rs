use serde::{Deserialize, Serialize};

const RELEASE_API: &str = "https://api.github.com/repos/yang102948549/oniscreen/releases/latest";
const DOWNLOAD_PREFIX: &str = "https://github.com/yang102948549/oniscreen/releases/download/";

#[derive(Deserialize)]
struct Asset { name: String, browser_download_url: String }
#[derive(Deserialize)]
struct Release { tag_name: String, assets: Vec<Asset> }
#[derive(Serialize)]
pub struct UpdateInfo { current: String, latest: String, available: bool }

fn newer(tag: &str, current: &str) -> Result<bool, String> {
    let latest = semver::Version::parse(tag.trim_start_matches('v')).map_err(|_| "릴리스 버전 형식이 올바르지 않습니다")?;
    let current = semver::Version::parse(current).map_err(|e| e.to_string())?;
    Ok(latest > current && latest.pre.is_empty())
}

async fn release() -> Result<Release, String> {
    let response = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20))
        .user_agent("OniScreen-update-check").build().map_err(|e| e.to_string())?
        .get(RELEASE_API).send().await.map_err(|_| "네트워크 연결을 확인해 주세요")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND { return Err("공개된 릴리스가 아직 없습니다".into()); }
    response.error_for_status().map_err(|e| format!("GitHub 응답 오류: {e}"))?
        .json().await.map_err(|_| "릴리스 정보를 읽지 못했습니다".into())
}

#[tauri::command]
pub async fn check_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let release = release().await?;
    let current = app.package_info().version.to_string();
    Ok(UpdateInfo { available: newer(&release.tag_name, &current)?, current, latest: release.tag_name })
}

#[tauri::command]
pub async fn download_update(app: tauri::AppHandle) -> Result<(), String> {
    let release = release().await?;
    if !newer(&release.tag_name, &app.package_info().version.to_string())? { return Err("이미 최신 버전입니다".into()); }
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    let asset = release.assets.iter().find(|a| a.name.ends_with("-setup.exe") && a.name.contains(arch))
        .ok_or("이 PC에 맞는 Windows 설치 파일이 아직 없습니다")?;
    let url = &asset.browser_download_url;
    if !url.starts_with(DOWNLOAD_PREFIX) || !url.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~:/%".contains(&b)) {
        return Err("설치 파일 주소가 올바르지 않습니다".into());
    }
    // Open only a validated release asset in the default browser; never execute a remote installer.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &format!("Start-Process '{}'", url)])
            .creation_flags(0x08000000).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_release_versions() {
        assert!(newer("v0.10.0", "0.9.0").unwrap());
        assert!(!newer("v0.1.0", "0.1.0").unwrap());
        assert!(!newer("v0.0.9", "0.1.0").unwrap());
        assert!(!newer("v1.0.0-beta.1", "0.1.0").unwrap());
        assert!(newer("invalid", "0.1.0").is_err());
    }
}
