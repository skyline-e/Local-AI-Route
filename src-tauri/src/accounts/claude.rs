use super::models::{AccountQuota, QuotaWindow};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const API_BASE: &str = "https://api.anthropic.com";

pub fn find_claude_data_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let local_p = Path::new(&local);
            // 1. Check %LOCALAPPDATA%\Claude-Data\*
            let claude_data = local_p.join("Claude-Data");
            if let Ok(entries) = std::fs::read_dir(&claude_data) {
                for e in entries.flatten() {
                    candidates.push(e.path());
                }
            }

            // 2. Check Store packages: %LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude
            let packages = local_p.join("Packages");
            if let Ok(entries) = std::fs::read_dir(&packages) {
                for e in entries.flatten() {
                    let fname = e.file_name().to_string_lossy().to_string();
                    if fname.to_lowercase().starts_with("claude_") {
                        let c = e.path().join("LocalCache").join("Roaming").join("Claude");
                        candidates.push(c);
                    }
                }
            }
        }

        if let Ok(appdata) = std::env::var("APPDATA") {
            candidates.push(Path::new(&appdata).join("Claude"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("Library/Application Support/Claude"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(config_dir) = dirs::config_dir() {
            candidates.push(config_dir.join("Claude"));
            candidates.push(config_dir.join("claude-desktop"));
        }
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".config").join("Claude"));
            candidates.push(home.join(".config").join("claude-desktop"));
        }
    }

    // Pick candidate with newest config.json
    let mut best: Option<PathBuf> = None;
    let mut newest_time = std::time::SystemTime::UNIX_EPOCH;

    for dir in candidates {
        let cfg = dir.join("config.json");
        if let Ok(meta) = std::fs::metadata(&cfg) {
            if let Ok(mod_time) = meta.modified() {
                if mod_time >= newest_time {
                    newest_time = mod_time;
                    best = Some(dir);
                }
            }
        }
    }

    best
}

pub fn get_claude_cookie_files(claude_dir: &Path) -> Vec<PathBuf> {
    let mut base = claude_dir.to_path_buf();
    let net = claude_dir.join("Network");
    if net.is_dir() {
        base = net;
    }
    vec![
        base.join("Cookies"),
        base.join("Cookies-journal"),
        base.join("Cookies-wal"),
        base.join("Cookies-shm"),
    ]
}

#[allow(unused_variables)]
pub fn load_claude_app_key(claude_dir: &Path) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "windows")]
    {
        use base64::Engine;
        let local_state_path = claude_dir.join("Local State");
        if !local_state_path.is_file() {
            return Err("Claude Local State file not found".into());
        }

        let content = std::fs::read_to_string(&local_state_path)
            .map_err(|e| format!("Failed to read Local State: {}", e))?;
        let val: Value = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse Local State JSON: {}", e))?;

        let b64_key = val
            .get("os_crypt")
            .and_then(|o| o.get("encrypted_key"))
            .and_then(|k| k.as_str())
            .ok_or_else(|| "No os_crypt.encrypted_key in Local State".to_string())?;

        let raw_blob = base64::engine::general_purpose::STANDARD
            .decode(b64_key)
            .map_err(|e| format!("Base64 decode failed for encrypted_key: {}", e))?;

        if raw_blob.len() < 5 || &raw_blob[..5] != b"DPAPI" {
            return Err("Expected DPAPI prefix in encrypted_key".into());
        }

        super::crypto::dpapi::unprotect_data(&raw_blob[5..])
    }

    #[cfg(not(target_os = "windows"))]
    {
        Err("Key extraction currently implemented for Windows".into())
    }
}

pub fn extract_access_token_from_token_cache(
    claude_dir: &Path,
    token_cache_v2_b64: &str,
    target_account_id: Option<&str>,
) -> Result<String, String> {
    use base64::Engine;
    let key = load_claude_app_key(claude_dir)?;
    let encrypted_bytes = base64::engine::general_purpose::STANDARD
        .decode(token_cache_v2_b64)
        .map_err(|e| format!("Base64 decode error for tokenCache: {}", e))?;

    let decrypted = super::crypto::decrypt_chromium_v10(&key, &encrypted_bytes)?;
    let cache: HashMap<String, Value> = serde_json::from_slice(&decrypted)
        .map_err(|e| format!("Failed to parse decrypted tokenCache JSON: {}", e))?;

    let now_ms = Utc::now().timestamp_millis();
    let mut best_token = String::new();
    let mut best_exp = 0i64;

    for (k, v) in cache {
        if let Some(target_id) = target_account_id {
            if !target_id.is_empty() && !k.contains(target_id) {
                continue;
            }
        }
        if !k.contains("user:profile") && !k.contains("user:inference") {
            continue;
        }

        let token = v.get("token").and_then(|t| t.as_str()).unwrap_or("");
        let exp = v.get("expiresAt").and_then(|e| e.as_i64()).unwrap_or(0);

        if exp > now_ms && exp > best_exp && !token.is_empty() {
            best_token = token.to_string();
            best_exp = exp;
        }
    }

    if best_token.is_empty() {
        return Err("No unexpired Claude access token found in tokenCache".into());
    }

    Ok(best_token)
}

#[derive(Debug, Deserialize)]
struct ClaudeProfileResponse {
    account: Option<ClaudeAccountInfo>,
    organization: Option<ClaudeOrgInfo>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ClaudeAccountInfo {
    uuid: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
    has_claude_max: Option<bool>,
    has_claude_pro: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ClaudeOrgInfo {
    organization_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeUsageResponse {
    five_hour: Option<ClaudeUsageWindow>,
    seven_day: Option<ClaudeUsageWindow>,
}

#[derive(Debug, Deserialize)]
struct ClaudeUsageWindow {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

pub async fn query_quota(
    claude_dir: &Path,
    auth_config_json: &str,
    account_id: &str,
    proxy: Option<&crate::core::models::ProxyConfig>,
) -> Result<AccountQuota, String> {
    let auth_val: Value = serde_json::from_str(auth_config_json)
        .map_err(|e| format!("Failed to parse Claude auth config: {}", e))?;

    let token_cache_v2 = auth_val
        .get("oauth:tokenCacheV2")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing oauth:tokenCacheV2 in Claude auth config".to_string())?;

    let uuid_val = auth_val
        .get("lastKnownAccountUuid")
        .and_then(|v| v.as_str())
        .unwrap_or(account_id);

    let access_token = if token_cache_v2.starts_with("sk-ant-") || token_cache_v2.starts_with("Bearer ") {
        token_cache_v2.trim_start_matches("Bearer ").trim().to_string()
    } else {
        extract_access_token_from_token_cache(claude_dir, token_cache_v2, Some(uuid_val))
            .or_else(|_| extract_access_token_from_token_cache(claude_dir, token_cache_v2, None))?
    };

    let client = super::build_http_client(proxy, 15);

    let mut email = None;
    let mut plan_type = None;
    let mut display_name = None;

    // 1. Fetch Profile
    let prof_res = client
        .get(format!("{}/api/oauth/profile", API_BASE))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", access_token))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header(reqwest::header::USER_AGENT, "claude-account-switcher/1.0")
        .send()
        .await;

    if let Ok(res) = prof_res {
        if res.status().is_success() {
            if let Ok(prof) = res.json::<ClaudeProfileResponse>().await {
                if let Some(acct) = prof.account {
                    email = acct.email;
                    display_name = acct.display_name;
                    if acct.has_claude_max == Some(true) {
                        plan_type = Some("Max".into());
                    } else if acct.has_claude_pro == Some(true) {
                        plan_type = Some("Pro".into());
                    }
                }
                if let Some(org) = prof.organization {
                    if let Some(org_t) = org.organization_type {
                        let trimmed = org_t.trim_start_matches("claude_");
                        if !trimmed.is_empty() {
                            let mut chars = trimmed.chars();
                            let capitalized = match chars.next() {
                                None => String::new(),
                                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                            };
                            plan_type = Some(capitalized);
                        }
                    }
                }
            }
        }
    }

    // 2. Fetch Usage
    let mut windows = Vec::new();
    let usage_res = client
        .get(format!("{}/api/oauth/usage", API_BASE))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", access_token))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header(reqwest::header::USER_AGENT, "claude-account-switcher/1.0")
        .send()
        .await;

    if let Ok(res) = usage_res {
        if res.status().is_success() {
            if let Ok(usage) = res.json::<ClaudeUsageResponse>().await {
                if let Some(w5) = usage.five_hour {
                    let used = w5.utilization.unwrap_or(0.0);
                    let rem = (100.0 - used).clamp(0.0, 100.0);
                    let (formatted, sec) = format_rfc3339(w5.resets_at.as_deref());

                    windows.push(QuotaWindow {
                        name: "5小时用量 (5-Hour Window)".into(),
                        used_percent: Some((used * 10.0).round() / 10.0),
                        remaining_percent: Some((rem * 10.0).round() / 10.0),
                        resets_at: formatted,
                        reset_in_seconds: sec,
                        detail: Some("滚动5小时会话限制 (Rolling 5h window)".into()),
                    });
                }

                if let Some(w7) = usage.seven_day {
                    let used = w7.utilization.unwrap_or(0.0);
                    let rem = (100.0 - used).clamp(0.0, 100.0);
                    let (formatted, sec) = format_rfc3339(w7.resets_at.as_deref());

                    windows.push(QuotaWindow {
                        name: "周用量限制 (Weekly Limit)".into(),
                        used_percent: Some((used * 10.0).round() / 10.0),
                        remaining_percent: Some((rem * 10.0).round() / 10.0),
                        resets_at: formatted,
                        reset_in_seconds: sec,
                        detail: Some("7天套餐额度限制 (7-day plan limit)".into()),
                    });
                }
            }
        }
    }

    let mut extra = HashMap::new();
    if let Some(name) = display_name {
        extra.insert("display_name".into(), name);
    }

    Ok(AccountQuota {
        provider: "claude".into(),
        account_id: account_id.to_string(),
        email,
        plan_type: plan_type.or(Some("Claude Pro".into())),
        fetched_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        windows,
        extra_info: extra,
    })
}

fn format_rfc3339(date_str: Option<&str>) -> (Option<String>, Option<i64>) {
    if let Some(s) = date_str {
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            let diff = dt.with_timezone(&Utc) - Utc::now();
            let sec = diff.num_seconds().max(0);
            let local_time = dt.with_timezone(&chrono::Local);
            return (Some(local_time.format("%Y-%m-%d %H:%M:%S").to_string()), Some(sec));
        }
    }
    (None, None)
}
