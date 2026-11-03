//! Self-update from GitHub Releases (public repo): find the newest release,
//! download its NSIS installer and run it.

use serde::Serialize;
use serde_json::Value;

const LATEST: &str = "https://api.github.com/repos/satthupc00/MTrans/releases/latest";

#[derive(Serialize, Clone, Default)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub has_update: bool,
    pub url: String,
    pub notes: String,
}

fn parse_ver(v: &str) -> Vec<u64> {
    v.trim_start_matches(['v', 'V'])
        .split(['.', '-', '+'])
        .take(3)
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

pub fn newer(latest: &str, current: &str) -> bool {
    parse_ver(latest) > parse_ver(current)
}

pub async fn check(current: &str) -> Result<UpdateInfo, String> {
    let resp = reqwest::Client::new()
        .get(LATEST)
        .header("User-Agent", "MTrans-updater")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| "Không kết nối được GitHub".to_string())?;
    if resp.status().as_u16() == 404 {
        // No release published yet.
        return Ok(UpdateInfo { current: current.into(), latest: current.into(), ..Default::default() });
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub trả lỗi {}", resp.status().as_u16()));
    }
    let v: Value = resp.json().await.map_err(|_| "Dữ liệu GitHub không đọc được".to_string())?;
    let latest = v["tag_name"].as_str().unwrap_or("").trim_start_matches('v').to_string();
    let url = v["assets"]
        .as_array()
        .and_then(|a| {
            a.iter().find_map(|x| {
                let name = x["name"].as_str()?.to_lowercase();
                (name.ends_with("-setup.exe")).then(|| x["browser_download_url"].as_str().map(String::from))?
            })
        })
        .unwrap_or_default();
    Ok(UpdateInfo {
        current: current.into(),
        has_update: !url.is_empty() && newer(&latest, current),
        latest,
        url,
        notes: v["body"].as_str().unwrap_or("").to_string(),
    })
}

/// Download the installer to the temp folder and start it. The caller exits the app.
pub async fn download_and_run(url: &str) -> Result<(), String> {
    if !url.starts_with("https://github.com/satthupc00/MTrans/") {
        return Err("Đường dẫn cập nhật không hợp lệ".into());
    }
    let resp = reqwest::Client::new()
        .get(url)
        .header("User-Agent", "MTrans-updater")
        .send()
        .await
        .map_err(|_| "Không tải được bản cập nhật".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Tải bản cập nhật lỗi {}", resp.status().as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|_| "Tải bản cập nhật bị gián đoạn".to_string())?;
    let path = std::env::temp_dir().join("MTrans-setup.exe");
    std::fs::write(&path, &bytes).map_err(|_| "Không lưu được file cài đặt".to_string())?;
    std::process::Command::new(&path)
        .spawn()
        .map_err(|_| "Không mở được file cài đặt".to_string())?;
    Ok(())
}
