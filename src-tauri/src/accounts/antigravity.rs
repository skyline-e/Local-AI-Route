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
                        // Struct for internal model quota calculation
                        struct ModelLimitEntry {
                            remaining_fraction: f64,
                            resets_at: Option<String>,
                            reset_in_seconds: Option<i64>,
                        }

                        let mut gemini_5h: Option<ModelLimitEntry> = None;
                        let mut gemini_weekly: Option<ModelLimitEntry> = None;
                        let mut claude_5h: Option<ModelLimitEntry> = None;
                        let mut claude_weekly: Option<ModelLimitEntry> = None;
                        let mut other_windows: Vec<QuotaWindow> = Vec::new();

                        let mut has_gemini = false;
                        let mut has_claude_gpt = false;

                        let mut sorted_keys: Vec<_> = models.keys().cloned().collect();
                        sorted_keys.sort();

                        for key in sorted_keys {
                            let key_lower = key.to_lowercase();
                            // Exclude internal tab/chat background models
                            if key_lower.starts_with("chat_") || key_lower.starts_with("tab_") {
                                continue;
                            }

                            let info = &models[&key];
                            let display = info.display_name.clone().unwrap_or_else(|| key.clone());

                            let (rem_frac, formatted_resets, reset_sec) = if let Some(q) = &info.quota_info {
                                let rem = q.remaining_fraction.unwrap_or(1.0);
                                let mut sec = None;
                                let mut formatted = None;
                                if let Some(ref rt) = q.reset_time {
                                    if let Ok(dt) = DateTime::parse_from_rfc3339(rt) {
                                        let diff = dt.with_timezone(&Utc) - Utc::now();
                                        let s = diff.num_seconds().max(0);
                                        sec = Some(s);
                                        let local_time = dt.with_timezone(&chrono::Local);
                                        formatted = Some(local_time.format("%Y-%m-%d %H:%M:%S").to_string());
                                    }
                                }
                                (rem, formatted, sec)
                            } else {
                                (1.0, None, None)
                            };

                            let is_argon = key_lower.contains("argon") || display.to_lowercase().contains("argon");

                            if !is_argon && key_lower.contains("gemini") {
                                has_gemini = true;
                                if let Some(sec) = reset_sec {
                                    if sec <= 86400 {
                                        // 5-hour rolling limit window
                                        if gemini_5h.as_ref().is_none_or(|e| rem_frac < e.remaining_fraction) {
                                            gemini_5h = Some(ModelLimitEntry {
                                                remaining_fraction: rem_frac,
                                                resets_at: formatted_resets,
                                                reset_in_seconds: Some(sec),
                                            });
                                        }
                                    } else {
                                        // Weekly limit window
                                        if gemini_weekly.as_ref().is_none_or(|e| rem_frac < e.remaining_fraction) {
                                            gemini_weekly = Some(ModelLimitEntry {
                                                remaining_fraction: rem_frac,
                                                resets_at: formatted_resets,
                                                reset_in_seconds: Some(sec),
                                            });
                                        }
                                    }
                                } else if gemini_5h.as_ref().is_none_or(|e| rem_frac < e.remaining_fraction) {
                                    gemini_5h = Some(ModelLimitEntry {
                                        remaining_fraction: rem_frac,
                                        resets_at: None,
                                        reset_in_seconds: None,
                                    });
                                }
                            } else if !is_argon && (key_lower.contains("claude") || key_lower.contains("gpt")) {
                                has_claude_gpt = true;
                                if let Some(sec) = reset_sec {
                                    if sec <= 86400 {
                                        // 5-hour rolling limit window
                                        if claude_5h.as_ref().is_none_or(|e| rem_frac < e.remaining_fraction) {
                                            claude_5h = Some(ModelLimitEntry {
                                                remaining_fraction: rem_frac,
                                                resets_at: formatted_resets,
                                                reset_in_seconds: Some(sec),
                                            });
                                        }
                                    } else {
                                        // Weekly limit window
                                        if claude_weekly.as_ref().is_none_or(|e| rem_frac < e.remaining_fraction) {
                                            claude_weekly = Some(ModelLimitEntry {
                                                remaining_fraction: rem_frac,
                                                resets_at: formatted_resets,
                                                reset_in_seconds: Some(sec),
                                            });
                                        }
                                    }
                                } else if claude_weekly.as_ref().is_none_or(|e| rem_frac < e.remaining_fraction) {
                                    claude_weekly = Some(ModelLimitEntry {
                                        remaining_fraction: rem_frac,
                                        resets_at: None,
                                        reset_in_seconds: None,
                                    });
                                }
                            } else {
                                // Specific model (such as Gemini 4 Argon or custom model)
                                let rem_pct = (rem_frac * 100.0).clamp(0.0, 100.0);
                                let used_pct = (100.0 - rem_pct).clamp(0.0, 100.0);
                                let win_type_suffix = if reset_sec.is_some_and(|s| s > 86400) {
                                    "周用量限制 (Weekly limit)"
                                } else {
                                    "5小时速率 (5h limit)"
                                };
                                other_windows.push(QuotaWindow {
                                    name: format!("{} · {}", display, win_type_suffix),
                                    used_percent: Some((used_pct * 10.0).round() / 10.0),
                                    remaining_percent: Some((rem_pct * 10.0).round() / 10.0),
                                    resets_at: formatted_resets,
                                    reset_in_seconds: reset_sec,
                                    detail: Some(format!("Model ID: {}", key)),
                                });
                            }
                        }

                        // Generate official Gemini Models quota pool windows (Image 3 layout)
                        if has_gemini {
                            // 1. Gemini Weekly limit
                            let (rem_pct, used_pct, resets_at, reset_sec, detail) = if let Some(w) = gemini_weekly {
                                let rem = (w.remaining_fraction * 100.0).clamp(0.0, 100.0);
                                (Some((rem * 10.0).round() / 10.0), Some(((100.0 - rem) * 10.0).round() / 10.0), w.resets_at, w.reset_in_seconds, "Gemini 深度思考与主力模型周总用量池")
                            } else {
                                (Some(100.0), Some(0.0), None, None, "Gemini 深度思考与主力模型周总用量池 (额度充足)")
                            };
                            windows.push(QuotaWindow {
                                name: "Gemini Models · 周用量限制 (Weekly limit)".into(),
                                used_percent: used_pct,
                                remaining_percent: rem_pct,
                                resets_at,
                                reset_in_seconds: reset_sec,
                                detail: Some(detail.into()),
                            });

                            // 2. Gemini 5h limit
                            let (rem_pct, used_pct, resets_at, reset_sec, detail) = if let Some(w) = gemini_5h {
                                let rem = (w.remaining_fraction * 100.0).clamp(0.0, 100.0);
                                (Some((rem * 10.0).round() / 10.0), Some(((100.0 - rem) * 10.0).round() / 10.0), w.resets_at, w.reset_in_seconds, "适用于 Gemini 2.5 Pro / Flash Lite / Flash 等全部 Gemini 模型")
                            } else {
                                (Some(100.0), Some(0.0), None, None, "适用于 Gemini 2.5 Pro / Flash 等全部 Gemini 模型 (速率充足)")
                            };
                            windows.push(QuotaWindow {
                                name: "Gemini Models · 5小时速率 (5h limit)".into(),
                                used_percent: used_pct,
                                remaining_percent: rem_pct,
                                resets_at,
                                reset_in_seconds: reset_sec,
                                detail: Some(detail.into()),
                            });
                        }

                        // Generate official Claude & GPT models quota pool windows (Image 3 layout)
                        if has_claude_gpt {
                            // 1. Claude/GPT Weekly limit
                            let (rem_pct, used_pct, resets_at, reset_sec, detail) = if let Some(w) = claude_weekly {
                                let rem = (w.remaining_fraction * 100.0).clamp(0.0, 100.0);
                                (Some((rem * 10.0).round() / 10.0), Some(((100.0 - rem) * 10.0).round() / 10.0), w.resets_at, w.reset_in_seconds, "适用于 Claude Opus 5.5 / Sonnet 5.5 及 GPT 等高阶模型")
                            } else {
                                (Some(100.0), Some(0.0), None, None, "高阶模型周总用量池 (额度充足)")
                            };
                            windows.push(QuotaWindow {
                                name: "Claude & GPT · 周用量限制 (Weekly limit)".into(),
                                used_percent: used_pct,
                                remaining_percent: rem_pct,
                                resets_at,
                                reset_in_seconds: reset_sec,
                                detail: Some(detail.into()),
                            });

                            // 2. Claude/GPT 5h limit
                            let (rem_pct, used_pct, resets_at, reset_sec, detail) = if let Some(w) = claude_5h {
                                let rem = (w.remaining_fraction * 100.0).clamp(0.0, 100.0);
                                (Some((rem * 10.0).round() / 10.0), Some(((100.0 - rem) * 10.0).round() / 10.0), w.resets_at, w.reset_in_seconds, "Claude / GPT 短周期突发速率窗口")
                            } else {
                                (Some(100.0), Some(0.0), None, None, "Claude / GPT 短周期突发速率窗口 (未受限/充足)")
                            };
                            windows.push(QuotaWindow {
                                name: "Claude & GPT · 5小时速率 (5h limit)".into(),
                                used_percent: used_pct,
                                remaining_percent: rem_pct,
                                resets_at,
                                reset_in_seconds: reset_sec,
                                detail: Some(detail.into()),
                            });
                        }

                        // Append any Argon or custom model windows
                        windows.extend(other_windows);
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

