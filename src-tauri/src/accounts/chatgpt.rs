use super::models::{AccountQuota, QuotaWindow};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

pub fn get_active_codex_home() -> PathBuf {
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".codex");
    }
    PathBuf::from(".codex")
}

pub fn get_active_auth_path() -> PathBuf {
    get_active_codex_home().join("auth.json")
}

pub fn read_current_auth_json() -> Result<String, String> {
    let path = get_active_auth_path();
    if !path.is_file() {
        return Err(format!("ChatGPT/Codex active auth file not found at: {}", path.display()));
    }
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read auth.json: {}", e))
}

pub fn write_auth_json(content: &str) -> Result<(), String> {
    let path = get_active_auth_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, content).map_err(|e| format!("Failed to write auth.json: {}", e))
}

pub struct DecodedJwtIdentity {
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub plan_type: Option<String>,
    pub active_until: Option<String>,
}

pub fn decode_jwt_claims(jwt: &str) -> Option<Value> {
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
    serde_json::from_slice(&decoded).ok()
}

pub fn parse_identity_from_auth(auth_json: &str) -> DecodedJwtIdentity {
    let mut identity = DecodedJwtIdentity {
        email: None,
        account_id: None,
        plan_type: None,
        active_until: None,
    };

    let val: Value = match serde_json::from_str(auth_json) {
        Ok(v) => v,
        Err(_) => return identity,
    };

    let tokens = val.get("tokens");
    if let Some(tokens_obj) = tokens {
        if let Some(aid) = tokens_obj.get("account_id").and_then(|v| v.as_str()) {
            identity.account_id = Some(aid.to_string());
        }

        if let Some(id_token) = tokens_obj.get("id_token").and_then(|v| v.as_str()) {
            if let Some(claims) = decode_jwt_claims(id_token) {
                if let Some(email) = claims.get("email").and_then(|e| e.as_str()) {
                    identity.email = Some(email.to_string());
                }

                if let Some(auth_claim) = claims.get("https://api.openai.com/auth") {
                    if identity.account_id.is_none() {
                        if let Some(aid) = auth_claim.get("chatgpt_account_id").and_then(|v| v.as_str()) {
                            identity.account_id = Some(aid.to_string());
                        }
                    }
                    if let Some(plan) = auth_claim.get("chatgpt_plan_type").and_then(|v| v.as_str()) {
                        identity.plan_type = Some(plan.to_string());
                    }
                    if let Some(until) = auth_claim.get("chatgpt_subscription_active_until").and_then(|v| v.as_str()) {
                        identity.active_until = Some(until.to_string());
                    }
                }
            }
        }
    }

    identity
}

pub async fn query_quota(
    auth_json: &str,
    account_id: &str,
    proxy: Option<&crate::core::models::ProxyConfig>,
) -> Result<AccountQuota, String> {
    let identity = parse_identity_from_auth(auth_json);
    let val: Value = serde_json::from_str(auth_json).unwrap_or(Value::Null);
    let access_token = val
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut windows = Vec::new();
    let mut extra_info = HashMap::new();

    // 1. Try querying chatgpt.com backend-api usage
    let mut _remote_fetched = false;
    if !access_token.is_empty() {
        let client = super::build_http_client(proxy, 12);

        let res = client
            .get("https://chatgpt.com/backend-api/wham/usage")
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", access_token))
            .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36")
            .send()
            .await;

        if let Ok(resp) = res {
            if resp.status().is_success() {
                if let Ok(data) = resp.json::<Value>().await {
                    _remote_fetched = true;
                    // Parse rate limits: support both snake_case (rate_limit) and camelCase (rateLimits)
                    let rate_limits_opt = data.get("rate_limit")
                        .or_else(|| data.get("rateLimits"))
                        .or_else(|| data.get("rateLimitsByLimitId").and_then(|m| m.get("codex")));

                    if let Some(rate_limits) = rate_limits_opt {
                        // Primary window (typically 5-hour rolling window)
                        if let Some(primary) = rate_limits.get("primary_window").or_else(|| rate_limits.get("primary")) {
                            let used = primary.get("used_percent")
                                .or_else(|| primary.get("usedPercent"))
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0);
                            let remaining = (100.0 - used).clamp(0.0, 100.0);

                            let resets_at_ts = primary.get("reset_at")
                                .or_else(|| primary.get("resetsAt"))
                                .and_then(|v| v.as_i64())
                                .or_else(|| {
                                    primary.get("reset_after_seconds")
                                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                                        .map(|s| Utc::now().timestamp() + s)
                                });
                            let (formatted, sec) = format_ts(resets_at_ts);

                            let limit_sec = primary.get("limit_window_seconds").and_then(|v| v.as_i64()).unwrap_or(18000);
                            let hours = ((limit_sec + 1800) / 3600).max(1);

                            windows.push(QuotaWindow {
                                name: format!("{}小时速率窗口 ({}-Hour Window)", hours, hours),
                                used_percent: Some((used * 10.0).round() / 10.0),
                                remaining_percent: Some((remaining * 10.0).round() / 10.0),
                                resets_at: formatted,
                                reset_in_seconds: sec,
                                detail: Some(format!("ChatGPT {} · {}小时滚动速率用量限制", identity.plan_type.as_deref().unwrap_or("Plus"), hours)),
                            });
                        }

                        // Secondary window (typically 7-day rolling window)
                        if let Some(secondary) = rate_limits.get("secondary_window").or_else(|| rate_limits.get("secondary")) {
                            let used = secondary.get("used_percent")
                                .or_else(|| secondary.get("usedPercent"))
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0);
                            let remaining = (100.0 - used).clamp(0.0, 100.0);

                            let resets_at_ts = secondary.get("reset_at")
                                .or_else(|| secondary.get("resetsAt"))
                                .and_then(|v| v.as_i64())
                                .or_else(|| {
                                    secondary.get("reset_after_seconds")
                                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                                        .map(|s| Utc::now().timestamp() + s)
                                });
                            let (formatted, sec) = format_ts(resets_at_ts);

                            let limit_sec = secondary.get("limit_window_seconds").and_then(|v| v.as_i64()).unwrap_or(604800);
                            let days = ((limit_sec + 43200) / 86400).max(1);

                            windows.push(QuotaWindow {
                                name: if days >= 7 { "周用量窗口 (Weekly Window)".into() } else { format!("{}天速率窗口 ({}-Day Window)", days, days) },
                                used_percent: Some((used * 10.0).round() / 10.0),
                                remaining_percent: Some((remaining * 10.0).round() / 10.0),
                                resets_at: formatted,
                                reset_in_seconds: sec,
                                detail: Some(format!("ChatGPT 每周总用量限制 ({}天滚动重置)", days)),
                            });
                        }
                    }
                }
            }
        }
    }

    // 2. Subscription expiration status & fallback card
    if let Some(ref until_str) = identity.active_until {
        let mut formatted_until = until_str.clone();
        let mut sec_remaining = None;
        if let Ok(dt) = DateTime::parse_from_rfc3339(until_str) {
            let diff = dt.with_timezone(&Utc) - Utc::now();
            sec_remaining = Some(diff.num_seconds().max(0));
            let local_time = dt.with_timezone(&chrono::Local);
            formatted_until = local_time.format("%Y-%m-%d %H:%M:%S").to_string();
        }

        extra_info.insert("subscription_expires_at".into(), formatted_until.clone());
        if let Some(sec) = sec_remaining {
            extra_info.insert("subscription_sec_remaining".into(), sec.to_string());
            extra_info.insert("subscription_is_expired".into(), if sec == 0 { "true" } else { "false" }.into());
        }

        // Only if remote usage returned no rate limits, provide subscription fallback card
        if windows.is_empty() {
            windows.push(QuotaWindow {
                name: "订阅有效期 (Subscription Active Until)".into(),
                used_percent: None,
                remaining_percent: None,
                resets_at: Some(formatted_until),
                reset_in_seconds: sec_remaining,
                detail: Some(format!("套餐: {}", identity.plan_type.as_deref().unwrap_or("ChatGPT Plus"))),
            });
        }
    } else if windows.is_empty() {
        windows.push(QuotaWindow {
            name: "ChatGPT 账号就绪 (Account Ready)".into(),
            used_percent: None,
            remaining_percent: None,
            resets_at: None,
            reset_in_seconds: None,
            detail: Some(format!("套餐: {}", identity.plan_type.as_deref().unwrap_or("Standard"))),
        });
    }

    if let Some(plan) = &identity.plan_type {
        extra_info.insert("plan_type".into(), plan.clone());
    }
    if let Some(aid) = &identity.account_id {
        extra_info.insert("account_id".into(), aid.clone());
    }

    Ok(AccountQuota {
        provider: "chatgpt".into(),
        account_id: account_id.to_string(),
        email: identity.email,
        plan_type: identity.plan_type.or(Some("ChatGPT Plus/Team".into())),
        fetched_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        windows,
        extra_info,
    })
}

fn format_ts(ts: Option<i64>) -> (Option<String>, Option<i64>) {
    if let Some(sec_ts) = ts {
        if let Some(dt) = DateTime::from_timestamp(sec_ts, 0) {
            let diff = dt - Utc::now();
            let sec = diff.num_seconds().max(0);
            let local_time = dt.with_timezone(&chrono::Local);
            return (Some(local_time.format("%Y-%m-%d %H:%M:%S").to_string()), Some(sec));
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_identity_from_auth() {
        let auth_json = r#"{
            "tokens": {
                "account_id": "org-test12345",
                "access_token": "eyTestAccessToken"
            }
        }"#;
        let id = parse_identity_from_auth(auth_json);
        assert_eq!(id.account_id, Some("org-test12345".to_string()));
    }
}

