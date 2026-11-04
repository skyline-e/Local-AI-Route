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
                    // Parse windows (5-hour, 7-day)
                    // Sample structures can have rateLimits or five_hour / seven_day
                    if let Some(rate_limits) = data.get("rateLimits").or_else(|| data.get("rateLimitsByLimitId").and_then(|m| m.get("codex"))) {
                        if let Some(primary) = rate_limits.get("primary") {
                            let used = primary.get("usedPercent").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let remaining = (100.0 - used).clamp(0.0, 100.0);
                            let resets_at_ts = primary.get("resetsAt").and_then(|v| v.as_i64());
                            let (formatted, sec) = format_ts(resets_at_ts);

                            windows.push(QuotaWindow {
                                name: "5小时窗口 (5-Hour Window)".into(),
                                used_percent: Some((used * 10.0).round() / 10.0),
                                remaining_percent: Some((remaining * 10.0).round() / 10.0),
                                resets_at: formatted,
                                reset_in_seconds: sec,
                                detail: Some("滚动短周期限制 (Short-term rolling limit)".into()),
                            });
                        }

                        if let Some(secondary) = rate_limits.get("secondary") {
                            let used = secondary.get("usedPercent").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let remaining = (100.0 - used).clamp(0.0, 100.0);
                            let resets_at_ts = secondary.get("resetsAt").and_then(|v| v.as_i64());
                            let (formatted, sec) = format_ts(resets_at_ts);

                            windows.push(QuotaWindow {
                                name: "周用量窗口 (Weekly Window)".into(),
                                used_percent: Some((used * 10.0).round() / 10.0),
                                remaining_percent: Some((remaining * 10.0).round() / 10.0),
                                resets_at: formatted,
                                reset_in_seconds: sec,
                                detail: Some("每周配额限制 (Weekly limit)".into()),
                            });
                        }
                    }
                }
            }
        }
    }

    // 2. If remote usage didn't succeed or provided no windows, provide subscription snapshot
    if windows.is_empty() {
        if let Some(ref until_str) = identity.active_until {
            let mut formatted_until = until_str.clone();
            let mut sec_remaining = None;
            if let Ok(dt) = DateTime::parse_from_rfc3339(until_str) {
                let diff = dt.with_timezone(&Utc) - Utc::now();
                sec_remaining = Some(diff.num_seconds().max(0));
                let local_time = dt.with_timezone(&chrono::Local);
                formatted_until = local_time.format("%Y-%m-%d %H:%M:%S").to_string();
            }

            windows.push(QuotaWindow {
                name: "订阅有效期 (Subscription Active Until)".into(),
                used_percent: None,
                remaining_percent: None,
                resets_at: Some(formatted_until),
                reset_in_seconds: sec_remaining,
                detail: Some(format!("套餐: {}", identity.plan_type.as_deref().unwrap_or("ChatGPT Plus"))),
            });
        } else {
            windows.push(QuotaWindow {
                name: "ChatGPT 账号就绪 (Account Ready)".into(),
                used_percent: None,
                remaining_percent: None,
                resets_at: None,
                reset_in_seconds: None,
                detail: Some(format!("套餐: {}", identity.plan_type.as_deref().unwrap_or("Standard"))),
            });
        }
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

