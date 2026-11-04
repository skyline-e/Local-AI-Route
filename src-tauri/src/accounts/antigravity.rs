use super::models::{AccountQuota, QuotaWindow};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

fn get_oauth_client_id() -> String {
    const RAW: &[u8] = &[
        107, 106, 109, 107, 106, 106, 108, 106, 108, 106, 111, 99, 107, 119, 46, 55, 50, 41, 41, 51,
        52, 104, 50, 104, 107, 54, 57, 40, 63, 104, 105, 111, 44, 46, 53, 54, 53, 48, 50, 110, 61,
        110, 106, 105, 63, 42, 116, 59, 42, 42, 41, 116, 61, 53, 53, 61, 54, 63, 47, 41, 63, 40, 57,
        53, 52, 46, 63, 52, 46, 116, 57, 53, 55,
    ];
    let decoded: Vec<u8> = RAW.iter().map(|b| b ^ 0x5a).collect();
    String::from_utf8(decoded).unwrap_or_default()
}

fn get_oauth_client_secret() -> String {
    const RAW: &[u8] = &[
        29, 21, 25, 9, 10, 2, 119, 17, 111, 98, 28, 13, 8, 110, 98, 108, 22, 62, 22, 16, 107, 55, 22,
        24, 98, 41, 2, 25, 110, 32, 108, 43, 30, 27, 60,
    ];
    let decoded: Vec<u8> = RAW.iter().map(|b| b ^ 0x5a).collect();
    String::from_utf8(decoded).unwrap_or_default()
}

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

const QUOTA_API_ENDPOINTS: [&str; 3] = [
    "https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
    "https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
    "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:fetchAvailableModels",
];

const LOAD_CODE_ASSIST_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";

pub fn get_cli_token_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let gemini_cli = home.join(".gemini").join("antigravity-cli").join("antigravity-oauth-token");
        paths.push(gemini_cli);
        let jetski = home.join(".gemini").join("jetski-standalone-oauth-token");
        paths.push(jetski);
    }
    paths
}

pub fn read_current_credential_payload() -> Result<String, String> {
    // 1. On Windows, try Credential Manager first
    #[cfg(target_os = "windows")]
    {
        if let Ok(bytes) = super::crypto::cred_mgr::read_credential("gemini:antigravity") {
            if let Ok(s) = String::from_utf8(bytes) {
                if !s.trim().is_empty() {
                    return Ok(s);
                }
            }
        }
    }

    // 2. Try CLI session files
    for path in get_cli_token_paths() {
        if path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if !content.trim().is_empty() {
                    return Ok(content);
                }
            }
        }
    }

    Err("No existing Antigravity credential found in system keyring or ~/.gemini/".into())
}

pub fn write_credential_payload(payload_json: &str) -> Result<(), String> {
    // 1. Write to Windows Credential Manager
    #[cfg(target_os = "windows")]
    {
        super::crypto::cred_mgr::write_credential(
            "gemini:antigravity",
            "antigravity",
            payload_json.as_bytes(),
        )?;
    }

    // 2. Write to CLI paths
    for path in get_cli_token_paths() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, payload_json);
    }

    Ok(())
}

pub fn parse_email_from_payload(payload_json: &str) -> Option<String> {
    let val: serde_json::Value = serde_json::from_str(payload_json).ok()?;
    
    // 1. Check id_token JWT
    if let Some(id_token) = val.get("id_token").and_then(|v| v.as_str()) {
        if let Some(email) = extract_email_from_jwt(id_token) {
            return Some(email);
        }
    }
    // 2. Check token object
    if let Some(email) = val.get("token").and_then(|t| t.get("email")).and_then(|e| e.as_str()) {
        return Some(email.to_string());
    }

    None
}

fn extract_email_from_jwt(jwt: &str) -> Option<String> {
    use base64::Engine;

    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    let payload = parts[1];
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| {
            let mut s = payload.replace('-', "+").replace('_', "/");
            let pad = (4 - (s.len() % 4)) % 4;
            s.push_str(&"=".repeat(pad));
            base64::engine::general_purpose::STANDARD.decode(s)
        })
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    claims.get("email").and_then(|e| e.as_str()).map(|s| s.to_string())
}

pub async fn refresh_access_token(
    refresh_token: &str,
    proxy: Option<&crate::core::models::ProxyConfig>,
) -> Result<(String, i64), String> {
    let client = super::build_http_client(proxy, 15);

    let client_id = get_oauth_client_id();
    let client_secret = get_oauth_client_secret();
    let params = [
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];

    let resp = client
        .post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Failed to send token refresh request: {}", e))?;

    if !resp.status().is_success() {
        let err_text = resp.text().await.unwrap_or_default();
        return Err(format!("Google token refresh failed: {}", err_text));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse refresh response: {}", e))?;

    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "No access_token in refresh response".to_string())?
        .to_string();

    let expires_in = json.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(3600);

    Ok((access_token, expires_in))
}

pub async fn ensure_fresh_access_token(
    payload: &mut serde_json::Value,
    proxy: Option<&crate::core::models::ProxyConfig>,
) -> Result<String, String> {
    let token_obj = payload
        .get_mut("token")
        .ok_or_else(|| "Missing token object in payload".to_string())?;

    let access_token = token_obj
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let refresh_token = token_obj
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let expiry_str = token_obj
        .get("expiry")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut is_expired = true;
    if let Ok(exp_time) = DateTime::parse_from_rfc3339(expiry_str) {
        if exp_time.with_timezone(&Utc) > Utc::now() + chrono::Duration::seconds(180) {
            is_expired = false;
        }
    }

    if !is_expired && !access_token.is_empty() {
        return Ok(access_token);
    }

    if refresh_token.is_empty() {
        if !access_token.is_empty() {
            return Ok(access_token);
        }
        return Err("No refresh_token or access_token available".into());
    }

    // Refresh token
    let (new_access, expires_in) = refresh_access_token(&refresh_token, proxy).await?;
    let new_expiry = (Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339();

    token_obj["access_token"] = json!(new_access);
    token_obj["expiry"] = json!(new_expiry);

    Ok(new_access)
}

#[derive(Debug, Deserialize)]
struct QuotaResponse {
    models: Option<HashMap<String, ModelInfo>>,
}

#[derive(Debug, Deserialize)]
struct ModelInfo {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "quotaInfo")]
    quota_info: Option<QuotaInfoItem>,
}

#[derive(Debug, Deserialize)]
struct QuotaInfoItem {
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
}

pub async fn query_quota(
    payload_json: &mut String,
    account_id: &str,
    proxy: Option<&crate::core::models::ProxyConfig>,
) -> Result<AccountQuota, String> {
    let mut val: serde_json::Value = serde_json::from_str(payload_json)
        .map_err(|e| format!("Invalid JSON payload: {}", e))?;

    let access_token = ensure_fresh_access_token(&mut val, proxy).await?;
    *payload_json = serde_json::to_string(&val).unwrap_or_else(|_| payload_json.clone());

    let email = parse_email_from_payload(payload_json);
    let client = super::build_http_client(proxy, 15);

    // 1. Fetch available models and quotas
    let mut windows = Vec::new();
    let mut plan_type = None;

    for ep in &QUOTA_API_ENDPOINTS {
        let resp = client
            .post(*ep)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", access_token))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::USER_AGENT, "antigravity-cli/1.0")
            .json(&json!({}))
            .send()
            .await;

        if let Ok(res) = resp {
            if res.status().is_success() {
                if let Ok(quota_data) = res.json::<QuotaResponse>().await {
                    if let Some(models) = quota_data.models {
                        // Sort models by key
                        let mut sorted_keys: Vec<_> = models.keys().cloned().collect();
                        sorted_keys.sort();

                        for key in sorted_keys {
                            let info = &models[&key];
                            let display = info.display_name.clone().unwrap_or_else(|| key.clone());

                            if let Some(q) = &info.quota_info {
                                let remaining = q.remaining_fraction.unwrap_or(1.0);
                                let remaining_pct = (remaining * 100.0).clamp(0.0, 100.0);
                                let used_pct = (100.0 - remaining_pct).clamp(0.0, 100.0);

                                let mut reset_sec = None;
                                let mut formatted_resets = None;
                                if let Some(ref rt) = q.reset_time {
                                    if let Ok(dt) = DateTime::parse_from_rfc3339(rt) {
                                        let diff = dt.with_timezone(&Utc) - Utc::now();
                                        let sec = diff.num_seconds().max(0);
                                        reset_sec = Some(sec);
                                        let local_time = dt.with_timezone(&chrono::Local);
                                        formatted_resets = Some(local_time.format("%Y-%m-%d %H:%M:%S").to_string());
                                    }
                                }

                                windows.push(QuotaWindow {
                                    name: display,
                                    used_percent: Some((used_pct * 10.0).round() / 10.0),
                                    remaining_percent: Some((remaining_pct * 10.0).round() / 10.0),
                                    resets_at: formatted_resets,
                                    reset_in_seconds: reset_sec,
                                    detail: Some(format!("Model ID: {}", key)),
                                });
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    // 2. Fetch Subscription tier via loadCodeAssist
    let tier_res = client
        .post(LOAD_CODE_ASSIST_ENDPOINT)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", access_token))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, "antigravity-cli/1.0")
        .json(&json!({"metadata": {"ideType": "ANTIGRAVITY"}}))
        .send()
        .await;

    if let Ok(res) = tier_res {
        if res.status().is_success() {
            if let Ok(tier_data) = res.json::<serde_json::Value>().await {
                if let Some(paid_tier) = tier_data.get("paidTier").and_then(|t| t.get("name")).and_then(|n| n.as_str()) {
                    plan_type = Some(paid_tier.to_string());
                } else if let Some(curr_tier) = tier_data.get("currentTier").and_then(|t| t.get("name")).and_then(|n| n.as_str()) {
                    plan_type = Some(curr_tier.to_string());
                }
            }
        }
    }

    Ok(AccountQuota {
        provider: "antigravity".into(),
        account_id: account_id.to_string(),
        email,
        plan_type,
        fetched_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        windows,
        extra_info: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_email_from_token_object() {
        let payload = r#"{
            "token": {
                "access_token": "ya29.test",
                "refresh_token": "1//test",
                "email": "developer@example.com"
            },
            "auth_method": "consumer"
        }"#;
        let email = parse_email_from_payload(payload);
        assert_eq!(email, Some("developer@example.com".to_string()));
    }
}

