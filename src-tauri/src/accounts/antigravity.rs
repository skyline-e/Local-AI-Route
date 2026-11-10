use super::models::{AccountQuota, QuotaWindow};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

pub(crate) fn get_oauth_client_id() -> String {
    const RAW: &[u8] = &[
        107, 106, 109, 107, 106, 106, 108, 106, 108, 106, 111, 99, 107, 119, 46, 55, 50, 41, 41, 51,
        52, 104, 50, 104, 107, 54, 57, 40, 63, 104, 105, 111, 44, 46, 53, 54, 53, 48, 50, 110, 61,
        110, 106, 105, 63, 42, 116, 59, 42, 42, 41, 116, 61, 53, 53, 61, 54, 63, 47, 41, 63, 40, 57,
        53, 52, 46, 63, 52, 46, 116, 57, 53, 55,
    ];
    let decoded: Vec<u8> = RAW.iter().map(|b| b ^ 0x5a).collect();
    String::from_utf8(decoded).unwrap_or_default()
}

pub(crate) fn get_oauth_client_secret() -> String {
    const RAW: &[u8] = &[
        29, 21, 25, 9, 10, 2, 119, 17, 111, 98, 28, 13, 8, 110, 98, 108, 22, 62, 22, 16, 107, 55, 22,
        24, 98, 41, 2, 25, 110, 32, 108, 43, 30, 27, 60,
    ];
    let decoded: Vec<u8> = RAW.iter().map(|b| b ^ 0x5a).collect();
    String::from_utf8(decoded).unwrap_or_default()
}

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// 官方 Antigravity IDE language_server 使用的 User-Agent。
/// 注意：`v1internal:retrieveUserQuotaSummary` 接口校验客户端标识（`aidev_client` 与 `cl` 版本号），
/// 若使用非官方 UA（如 `antigravity-cli/1.0`），后端会省略 `Gemini Models` 配额分组。
const OFFICIAL_USER_AGENT: &str =
    "antigravity/1.11.0 (aidev_client; os_type=windows; arch=amd64; cl=1013060670)";

const QUOTA_SUMMARY_ENDPOINTS: [&str; 3] = [
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
    "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
    "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:retrieveUserQuotaSummary",
];

const QUOTA_API_ENDPOINTS: [&str; 3] = [
    "https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
    "https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
    "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:fetchAvailableModels",
];

const LOAD_CODE_ASSIST_ENDPOINTS: [&str; 3] = [
    "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist",
    "https://daily-cloudcode-pa.googleapis.com/v1internal:loadCodeAssist",
    "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:loadCodeAssist",
];

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
    
    // 1. Direct top-level fields (e.g. Antigravity stores "id_token_email")
    for key in &["id_token_email", "email", "user_email"] {
        if let Some(em) = val.get(key).and_then(|v| v.as_str()) {
            let trimmed = em.trim();
            if !trimmed.is_empty() && trimmed.contains('@') {
                return Some(trimmed.to_string());
            }
        }
    }

    // 2. Inside token object
    if let Some(token) = val.get("token") {
        for key in &["id_token_email", "email", "user_email"] {
            if let Some(em) = token.get(key).and_then(|v| v.as_str()) {
                let trimmed = em.trim();
                if !trimmed.is_empty() && trimmed.contains('@') {
                    return Some(trimmed.to_string());
                }
            }
        }
        if let Some(id_token) = token.get("id_token").and_then(|v| v.as_str()) {
            if let Some(email) = extract_email_from_jwt(id_token) {
                return Some(email);
            }
        }
    }

    // 3. Top-level id_token JWT
    if let Some(id_token) = val.get("id_token").and_then(|v| v.as_str()) {
        if let Some(email) = extract_email_from_jwt(id_token) {
            return Some(email);
        }
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
struct QuotaSummaryResponse {
    groups: Option<Vec<QuotaSummaryGroup>>,
}

#[derive(Debug, Deserialize)]
struct QuotaSummaryGroup {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    buckets: Option<Vec<QuotaSummaryBucket>>,
}

#[derive(Debug, Deserialize)]
struct QuotaSummaryBucket {
    #[serde(rename = "bucketId")]
    bucket_id: Option<String>,
    window: Option<String>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
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

#[derive(Debug, Clone)]
struct ModelLimitEntry {
    remaining_fraction: f64,
    resets_at: Option<String>,
    reset_in_seconds: Option<i64>,
}

fn parse_reset_time(rt_opt: Option<&str>, now: DateTime<Utc>) -> (Option<String>, Option<i64>) {
    if let Some(rt) = rt_opt {
        if let Ok(dt) = DateTime::parse_from_rfc3339(rt) {
            let diff = dt.with_timezone(&Utc) - now;
            let s = diff.num_seconds().max(0);
            let local_time = dt.with_timezone(&chrono::Local);
            let formatted = local_time.format("%Y-%m-%d %H:%M:%S").to_string();
            return (Some(formatted), Some(s));
        }
    }
    (None, None)
}

/// 解析剩余额度比例。
/// Proto3 在序列化 JSON 时会省略默认值 0.0 的 `remainingFraction` 字段；
/// 因此当 `remainingFraction` 为 `None` 且存在 `resetTime` 时，说明额度已耗尽（0.0），
/// 当两者均为 `None` 时，说明该配额窗口尚未触发限流（1.0）。
fn resolve_remaining_fraction(remaining_fraction: Option<f64>, reset_time: Option<&str>) -> f64 {
    match remaining_fraction {
        Some(v) => v.clamp(0.0, 1.0),
        None if reset_time.is_some() => 0.0,
        None => 1.0,
    }
}

/// 将剩余比例（0.0..=1.0）按官方 Usage 面板规则四舍五入为整数百分比（0.0..=100.0）。
fn round_quota_percent(remaining_fraction: f64) -> (f64, f64) {
    let rem = (remaining_fraction * 100.0).round().clamp(0.0, 100.0);
    let used = (100.0 - rem).clamp(0.0, 100.0);
    (rem, used)
}

/// 当同一配额池存在多个子窗口或模型时，取剩余比例最小（最受限）的项；
/// 若剩余比例相同，则优先保留包含有效或更晚重置时间的项。
fn update_min_entry(target: &mut Option<ModelLimitEntry>, candidate: ModelLimitEntry) {
    let should_replace = match target.as_ref() {
        None => true,
        Some(existing) => {
            if candidate.remaining_fraction < existing.remaining_fraction - f64::EPSILON {
                true
            } else if (candidate.remaining_fraction - existing.remaining_fraction).abs()
                <= f64::EPSILON
            {
                candidate.reset_in_seconds.unwrap_or(-1) > existing.reset_in_seconds.unwrap_or(-1)
            } else {
                false
            }
        }
    };
    if should_replace {
        *target = Some(candidate);
    }
}

fn build_pool_window(
    name: &str,
    entry: Option<ModelLimitEntry>,
    active_detail: &str,
    idle_detail: &str,
) -> QuotaWindow {
    let (rem_pct, used_pct, resets_at, reset_sec, detail) = if let Some(w) = entry {
        let (rem, used) = round_quota_percent(w.remaining_fraction);
        (
            Some(rem),
            Some(used),
            w.resets_at,
            w.reset_in_seconds,
            active_detail,
        )
    } else {
        (Some(100.0), Some(0.0), None, None, idle_detail)
    };
    QuotaWindow {
        name: name.into(),
        used_percent: used_pct,
        remaining_percent: rem_pct,
        resets_at,
        reset_in_seconds: reset_sec,
        detail: Some(detail.into()),
    }
}

#[derive(Default)]
struct ParsedQuotaPools {
    gemini_weekly: Option<ModelLimitEntry>,
    gemini_5h: Option<ModelLimitEntry>,
    claude_weekly: Option<ModelLimitEntry>,
    claude_5h: Option<ModelLimitEntry>,
    argon_weekly: Option<ModelLimitEntry>,
    argon_5h: Option<ModelLimitEntry>,
    other_windows: Vec<QuotaWindow>,
    has_gemini: bool,
    has_claude_gpt: bool,
    has_argon: bool,
}

impl ParsedQuotaPools {
    fn is_missing_any_main_pool(&self) -> bool {
        self.gemini_weekly.is_none()
            || self.gemini_5h.is_none()
            || self.claude_weekly.is_none()
            || self.claude_5h.is_none()
            || (self.has_argon && self.argon_weekly.is_none())
    }

    fn upsert_other_window(&mut self, candidate: QuotaWindow) {
        if let Some(existing) = self
            .other_windows
            .iter_mut()
            .find(|w| w.name == candidate.name)
        {
            let cand_rem = candidate.remaining_percent.unwrap_or(100.0);
            let exist_rem = existing.remaining_percent.unwrap_or(100.0);
            let should_replace = if cand_rem < exist_rem - f64::EPSILON {
                true
            } else if (cand_rem - exist_rem).abs() <= f64::EPSILON {
                candidate.reset_in_seconds.unwrap_or(-1) > existing.reset_in_seconds.unwrap_or(-1)
            } else {
                false
            };
            if should_replace {
                *existing = candidate;
            }
        } else {
            self.other_windows.push(candidate);
        }
    }

    fn apply_quota_summary(&mut self, summary_data: QuotaSummaryResponse, now: DateTime<Utc>) {
        let Some(groups) = summary_data.groups else {
            return;
        };

        for group in groups {
            let raw_group_name = group.display_name.unwrap_or_default();
            let group_name = raw_group_name.to_lowercase();
            // 提前标记分组存在性，兼容 Proto3 在未触发限流时省略空 buckets 字段的场景
            if group_name.contains("argon") {
                self.has_argon = true;
            } else if group_name.contains("gemini") {
                self.has_gemini = true;
            } else if group_name.contains("claude") || group_name.contains("gpt") {
                self.has_claude_gpt = true;
            }

            let Some(buckets) = group.buckets else {
                continue;
            };

            for bucket in buckets {
                let raw_bucket_id = bucket.bucket_id.unwrap_or_default();
                let bucket_id = raw_bucket_id.to_lowercase();

                let window = bucket.window.unwrap_or_default().to_lowercase();
                let rem_frac =
                    resolve_remaining_fraction(bucket.remaining_fraction, bucket.reset_time.as_deref());
                let (resets_at, reset_sec) = parse_reset_time(bucket.reset_time.as_deref(), now);

                // Gemini 4 Argon 为独立配额池，需优先于普通 Gemini 分组判定（其 bucketId 形如 gemini-argon-wtu-weekly）
                let is_argon = bucket_id.contains("argon") || group_name.contains("argon");
                let is_gemini =
                    !is_argon && (bucket_id.starts_with("gemini-") || group_name.contains("gemini"));
                let is_claude_gpt = !is_argon
                    && (bucket_id.starts_with("3p-")
                        || group_name.contains("claude")
                        || group_name.contains("gpt"));

                // 优先以显式的 window / bucket_id 后缀判定周期，仅在两者均未指明时用 reset_sec 兜底
                let explicit_weekly = window == "weekly" || bucket_id.ends_with("-weekly");
                let explicit_5h = window == "5h" || bucket_id.ends_with("-5h");
                let is_weekly =
                    explicit_weekly || (!explicit_5h && reset_sec.is_some_and(|s| s > 86400));
                let is_5h =
                    explicit_5h || (!explicit_weekly && reset_sec.is_some_and(|s| s <= 86400));

                let entry = ModelLimitEntry {
                    remaining_fraction: rem_frac,
                    resets_at: resets_at.clone(),
                    reset_in_seconds: reset_sec,
                };

                if is_argon {
                    self.has_argon = true;
                    // Argon 官方默认提供周限额池；仅当显式声明 5h 窗口时归入 argon_5h，防止周限额剩余不足 24h 时被误判为 5h
                    if explicit_5h {
                        update_min_entry(&mut self.argon_5h, entry);
                    } else {
                        update_min_entry(&mut self.argon_weekly, entry);
                    }
                } else if is_gemini {
                    self.has_gemini = true;
                    if is_weekly {
                        update_min_entry(&mut self.gemini_weekly, entry);
                    } else if is_5h {
                        update_min_entry(&mut self.gemini_5h, entry);
                    }
                } else if is_claude_gpt {
                    self.has_claude_gpt = true;
                    if is_weekly {
                        update_min_entry(&mut self.claude_weekly, entry);
                    } else if is_5h {
                        update_min_entry(&mut self.claude_5h, entry);
                    }
                } else {
                    let (rem_pct, used_pct) = round_quota_percent(rem_frac);
                    let win_type_suffix = if is_weekly { "周用量限制" } else { "5小时速率" };
                    let display = if !raw_group_name.is_empty() {
                        raw_group_name.as_str()
                    } else {
                        raw_bucket_id.as_str()
                    };
                    self.upsert_other_window(QuotaWindow {
                        name: format!("{} · {}", display, win_type_suffix),
                        used_percent: Some(used_pct),
                        remaining_percent: Some(rem_pct),
                        resets_at,
                        reset_in_seconds: reset_sec,
                        detail: Some(format!("Bucket ID: {}", raw_bucket_id)),
                    });
                }
            }
        }
    }

    fn apply_available_models_fallback(&mut self, quota_data: QuotaResponse, now: DateTime<Utc>) {
        let Some(models) = quota_data.models else {
            return;
        };

        let mut fb_gemini_5h: Option<ModelLimitEntry> = None;
        let mut fb_gemini_weekly: Option<ModelLimitEntry> = None;
        let mut fb_claude_5h: Option<ModelLimitEntry> = None;
        let mut fb_claude_weekly: Option<ModelLimitEntry> = None;
        let mut fb_argon_weekly: Option<ModelLimitEntry> = None;

        let mut sorted_keys: Vec<_> = models.keys().cloned().collect();
        sorted_keys.sort();

        for key in sorted_keys {
            let key_lower = key.to_lowercase();
            // 排除内部补全/对话后台模型，保持与摘要接口一致
            if key_lower.starts_with("chat_") || key_lower.starts_with("tab_") {
                continue;
            }

            let info = &models[&key];
            let display = info.display_name.clone().unwrap_or_else(|| key.clone());
            let display_lower = display.to_lowercase();
            let is_argon = key_lower.contains("argon") || display_lower.contains("argon");

            let (rem_frac, formatted_resets, reset_sec) = if let Some(q) = &info.quota_info {
                let rem = resolve_remaining_fraction(q.remaining_fraction, q.reset_time.as_deref());
                let (formatted, sec) = parse_reset_time(q.reset_time.as_deref(), now);
                (rem, formatted, sec)
            } else {
                (1.0, None, None)
            };

            let entry = ModelLimitEntry {
                remaining_fraction: rem_frac,
                resets_at: formatted_resets.clone(),
                reset_in_seconds: reset_sec,
            };

            if is_argon {
                self.has_argon = true;
                update_min_entry(&mut fb_argon_weekly, entry);
            } else if key_lower.contains("gemini") {
                self.has_gemini = true;
                if let Some(sec) = reset_sec {
                    if sec <= 86400 {
                        update_min_entry(&mut fb_gemini_5h, entry);
                    } else {
                        update_min_entry(&mut fb_gemini_weekly, entry);
                    }
                } else {
                    update_min_entry(&mut fb_gemini_5h, entry);
                }
            } else if key_lower.contains("claude") || key_lower.contains("gpt") {
                self.has_claude_gpt = true;
                if let Some(sec) = reset_sec {
                    if sec <= 86400 {
                        update_min_entry(&mut fb_claude_5h, entry);
                    } else {
                        update_min_entry(&mut fb_claude_weekly, entry);
                    }
                } else {
                    update_min_entry(&mut fb_claude_weekly, entry);
                }
            } else {
                let (rem_pct, used_pct) = round_quota_percent(rem_frac);
                let win_type_suffix = if reset_sec.is_some_and(|s| s > 86400) {
                    "周用量限制"
                } else {
                    "5小时速率"
                };
                self.upsert_other_window(QuotaWindow {
                    name: format!("{} · {}", display, win_type_suffix),
                    used_percent: Some(used_pct),
                    remaining_percent: Some(rem_pct),
                    resets_at: formatted_resets,
                    reset_in_seconds: reset_sec,
                    detail: Some(format!("Model ID: {}", key)),
                });
            }
        }

        if self.gemini_5h.is_none() {
            self.gemini_5h = fb_gemini_5h;
        }
        if self.gemini_weekly.is_none() {
            self.gemini_weekly = fb_gemini_weekly;
        }
        if self.claude_5h.is_none() {
            self.claude_5h = fb_claude_5h;
        }
        if self.claude_weekly.is_none() {
            self.claude_weekly = fb_claude_weekly;
        }
        if self.argon_weekly.is_none() {
            self.argon_weekly = fb_argon_weekly;
        }
    }

    fn into_windows(self) -> Vec<QuotaWindow> {
        let mut windows = Vec::new();

        // 1. 生成官方 Gemini Models 配额窗口
        if self.has_gemini || self.gemini_weekly.is_some() || self.gemini_5h.is_some() {
            windows.push(build_pool_window(
                "Gemini Models · 周用量限制",
                self.gemini_weekly,
                "Gemini 深度思考与主力模型周总用量池",
                "Gemini 深度思考与主力模型周总用量池 (额度充足)",
            ));
            windows.push(build_pool_window(
                "Gemini Models · 5小时速率",
                self.gemini_5h,
                "适用于 Gemini 2.5 Pro / Flash Lite / Flash 等全部 Gemini 模型",
                "适用于 Gemini 2.5 Pro / Flash 等全部 Gemini 模型 (速率充足)",
            ));
        }

        // 2. 生成官方 Claude & GPT models 配额窗口
        if self.has_claude_gpt || self.claude_weekly.is_some() || self.claude_5h.is_some() {
            windows.push(build_pool_window(
                "Claude & GPT · 周用量限制",
                self.claude_weekly,
                "适用于 Claude Opus / Sonnet 及 GPT 等高阶模型",
                "高阶模型周总用量池 (额度充足)",
            ));
            windows.push(build_pool_window(
                "Claude & GPT · 5小时速率",
                self.claude_5h,
                "Claude / GPT 短周期突发速率窗口",
                "Claude / GPT 短周期突发速率窗口 (未受限/充足)",
            ));
        }

        // 3. 生成官方 Gemini 4 Argon 独立配额窗口
        if self.has_argon || self.argon_weekly.is_some() || self.argon_5h.is_some() {
            windows.push(build_pool_window(
                "Gemini 4 Argon · 周用量限制",
                self.argon_weekly,
                "Gemini 4 Argon 独立周总用量池",
                "Gemini 4 Argon 独立周总用量池 (额度充足)",
            ));

            if let Some(w) = self.argon_5h {
                windows.push(build_pool_window(
                    "Gemini 4 Argon · 5小时速率",
                    Some(w),
                    "Gemini 4 Argon 短周期突发速率窗口",
                    "Gemini 4 Argon 短周期突发速率窗口 (未受限/充足)",
                ));
            }
        }

        windows.extend(self.other_windows);
        windows
    }
}

async fn post_cloudcode_json(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
    body: &serde_json::Value,
) -> Result<reqwest::Response, String> {
    client
        .post(url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", access_token))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, OFFICIAL_USER_AGENT)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Request to {} failed: {}", url, e))
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

    let mut email = parse_email_from_payload(payload_json);
    let client = super::build_http_client(proxy, 15);

    // Fallback: If email is missing, fetch from Google OAuth userinfo endpoint
    if email.is_none() && !access_token.is_empty() {
        if let Ok(u_resp) = client
            .get("https://www.googleapis.com/oauth2/v3/userinfo")
            .bearer_auth(&access_token)
            .send()
            .await
        {
            if u_resp.status().is_success() {
                if let Ok(uinfo) = u_resp.json::<serde_json::Value>().await {
                    if let Some(em) = uinfo.get("email").and_then(|e| e.as_str()) {
                        email = Some(em.to_string());
                        val["id_token_email"] = serde_json::Value::String(em.to_string());
                        *payload_json = serde_json::to_string(&val).unwrap_or_else(|_| payload_json.clone());
                    }
                }
            }
        }
    }

    // 1. Fetch Subscription tier and companion project ID via loadCodeAssist
    let mut plan_type = None;
    let mut companion_project: Option<String> = None;
    let load_body = json!({"metadata": {"ideType": "ANTIGRAVITY"}});

    for ep in &LOAD_CODE_ASSIST_ENDPOINTS {
        if let Ok(res) = post_cloudcode_json(&client, ep, &access_token, &load_body).await {
            if res.status().is_success() {
                if let Ok(tier_data) = res.json::<serde_json::Value>().await {
                    if let Some(paid_tier) = tier_data.get("paidTier").and_then(|t| t.get("name")).and_then(|n| n.as_str()) {
                        plan_type = Some(paid_tier.to_string());
                    } else if let Some(curr_tier) = tier_data.get("currentTier").and_then(|t| t.get("name")).and_then(|n| n.as_str()) {
                        plan_type = Some(curr_tier.to_string());
                    }
                    if let Some(proj) = tier_data.get("cloudaicompanionProject").and_then(|p| p.as_str()) {
                        if !proj.trim().is_empty() {
                            companion_project = Some(proj.trim().to_string());
                        }
                    }
                    break;
                }
            }
        }
    }

    let now = Utc::now();
    let mut pools = ParsedQuotaPools::default();
    let mut last_error: Option<String> = None;

    // 2. Fetch official grouped quota summary (retrieveUserQuotaSummary) used by Antigravity IDE Usage panel
    let summary_body = match &companion_project {
        Some(proj) => json!({ "project": proj }),
        None => json!({}),
    };

    for ep in &QUOTA_SUMMARY_ENDPOINTS {
        match post_cloudcode_json(&client, ep, &access_token, &summary_body).await {
            Ok(res) if res.status().is_success() => match res.json::<QuotaSummaryResponse>().await {
                Ok(summary_data) if summary_data.groups.is_some() => {
                    pools.apply_quota_summary(summary_data, now);
                    break;
                }
                Ok(_) => {}
                Err(err) => {
                    last_error = Some(format!("Failed to parse quota summary from {}: {}", ep, err));
                }
            },
            Ok(res) => {
                last_error = Some(format!("HTTP {} from {}", res.status(), ep));
            }
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    // 3. Fallback / supplement via fetchAvailableModels if any main pool was not returned in retrieveUserQuotaSummary
    if pools.is_missing_any_main_pool() {
        for ep in &QUOTA_API_ENDPOINTS {
            match post_cloudcode_json(&client, ep, &access_token, &summary_body).await {
                Ok(res) if res.status().is_success() => match res.json::<QuotaResponse>().await {
                    Ok(quota_data) if quota_data.models.is_some() => {
                        pools.apply_available_models_fallback(quota_data, now);
                        break;
                    }
                    Ok(_) => {}
                    Err(err) => {
                        last_error = Some(format!("Failed to parse available models from {}: {}", ep, err));
                    }
                },
                Ok(res) => {
                    last_error = Some(format!("HTTP {} from {}", res.status(), ep));
                }
                Err(err) => {
                    last_error = Some(err);
                }
            }
        }
    }

    let windows = pools.into_windows();
    if windows.is_empty() {
        if let Some(err) = last_error {
            return Err(format!("Failed to query Antigravity quota: {}", err));
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

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-06T10:36:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

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

    #[test]
    fn test_quota_summary_parsing_and_rounding() {
        let raw_summary = r#"{
            "groups": [
                {
                    "displayName": "Gemini Models",
                    "buckets": [
                        {
                            "bucketId": "gemini-weekly",
                            "window": "weekly",
                            "resetTime": "2026-11-12T03:40:51Z",
                            "remainingFraction": 0.6712
                        },
                        {
                            "bucketId": "gemini-5h",
                            "window": "5h",
                            "resetTime": "2026-11-06T12:35:34Z",
                            "remainingFraction": 0.996
                        }
                    ]
                },
                {
                    "displayName": "Claude and GPT models",
                    "buckets": [
                        {
                            "bucketId": "3p-weekly",
                            "window": "weekly",
                            "resetTime": "2026-11-13T10:38:40Z",
                            "remainingFraction": 0.4681564
                        },
                        {
                            "bucketId": "3p-5h",
                            "window": "5h",
                            "resetTime": "2026-11-06T15:38:40Z",
                            "remainingFraction": 0
                        }
                    ]
                },
                {
                    "displayName": "Gemini 4 Argon",
                    "buckets": [
                        {
                            "bucketId": "gemini-argon-wtu-weekly",
                            "window": "weekly",
                            "resetTime": "2026-11-13T11:41:05Z",
                            "remainingFraction": 0.8712
                        }
                    ]
                }
            ]
        }"#;

        let now = fixed_now();
        let summary: QuotaSummaryResponse = serde_json::from_str(raw_summary).unwrap();
        let mut pools = ParsedQuotaPools::default();
        pools.apply_quota_summary(summary, now);
        assert!(!pools.is_missing_any_main_pool());

        let windows = pools.into_windows();
        assert_eq!(windows.len(), 5);
        assert_eq!(windows[0].name, "Gemini Models · 周用量限制");
        assert_eq!(windows[0].remaining_percent, Some(67.0));
        assert_eq!(windows[0].used_percent, Some(33.0));
        assert_eq!(windows[0].reset_in_seconds, Some(493491));

        assert_eq!(windows[1].name, "Gemini Models · 5小时速率");
        assert_eq!(windows[1].remaining_percent, Some(100.0));
        assert_eq!(windows[1].used_percent, Some(0.0));
        assert_eq!(windows[1].reset_in_seconds, Some(7174));

        assert_eq!(windows[2].name, "Claude & GPT · 周用量限制");
        assert_eq!(windows[2].remaining_percent, Some(47.0));
        assert_eq!(windows[2].used_percent, Some(53.0));
        assert_eq!(windows[2].reset_in_seconds, Some(604960));

        assert_eq!(windows[3].name, "Claude & GPT · 5小时速率");
        assert_eq!(windows[3].remaining_percent, Some(0.0));
        assert_eq!(windows[3].used_percent, Some(100.0));
        assert_eq!(windows[3].reset_in_seconds, Some(18160));

        assert_eq!(windows[4].name, "Gemini 4 Argon · 周用量限制");
        assert_eq!(windows[4].remaining_percent, Some(87.0));
        assert_eq!(windows[4].used_percent, Some(13.0));
        assert_eq!(windows[4].reset_in_seconds, Some(608705));
    }

    #[test]
    fn test_fallback_handles_proto3_omitted_zero_and_preserves_summary_pools() {
        let raw_summary = r#"{
            "groups": [
                {
                    "displayName": "Claude and GPT models",
                    "buckets": [
                        {
                            "bucketId": "3p-weekly",
                            "window": "weekly",
                            "resetTime": "2026-11-13T10:38:40Z",
                            "remainingFraction": 0.47
                        }
                    ]
                },
                {
                    "displayName": "Gemini 4 Argon"
                }
            ]
        }"#;
        let raw_models = r#"{
            "models": {
                "claude-sonnet-4-6": {
                    "displayName": "Claude Sonnet 4.6",
                    "quotaInfo": {
                        "resetTime": "2026-11-06T15:38:40Z"
                    }
                },
                "gemini-3-pro-high": {
                    "displayName": "Gemini 3.1 Pro (High)",
                    "quotaInfo": {
                        "remainingFraction": 0.996,
                        "resetTime": "2026-11-06T12:35:34Z"
                    }
                },
                "gemini-4-argon-medium": {
                    "displayName": "Gemini 4 Argon (Medium)",
                    "quotaInfo": {
                        "remainingFraction": 0.8712,
                        "resetTime": "2026-11-13T11:41:05Z"
                    }
                }
            }
        }"#;

        let now = fixed_now();
        let summary: QuotaSummaryResponse = serde_json::from_str(raw_summary).unwrap();
        let models: QuotaResponse = serde_json::from_str(raw_models).unwrap();

        let mut pools = ParsedQuotaPools::default();
        pools.apply_quota_summary(summary, now);
        assert!(pools.has_argon);
        assert!(pools.is_missing_any_main_pool());
        pools.apply_available_models_fallback(models, now);

        let windows = pools.into_windows();
        assert_eq!(windows.len(), 5);
        assert_eq!(windows[0].remaining_percent, Some(100.0));
        assert_eq!(windows[1].remaining_percent, Some(100.0));
        assert_eq!(windows[1].reset_in_seconds, Some(7174));
        // Claude 周限额保留自 summary (47%)，5h 限额由 fallback 补齐且识别 proto3 缺省 0%
        assert_eq!(windows[2].remaining_percent, Some(47.0));
        assert_eq!(windows[2].reset_in_seconds, Some(604960));
        assert_eq!(windows[3].remaining_percent, Some(0.0));
        assert_eq!(windows[3].used_percent, Some(100.0));
        assert_eq!(windows[3].reset_in_seconds, Some(18160));
        // Gemini 4 Argon 由 fallback 补齐且不污染 Gemini Models 周限额
        assert_eq!(windows[4].name, "Gemini 4 Argon · 周用量限制");
        assert_eq!(windows[4].remaining_percent, Some(87.0));
        assert_eq!(windows[4].used_percent, Some(13.0));
        assert_eq!(windows[4].reset_in_seconds, Some(608705));
    }

    #[test]
    fn test_tie_breaking_prefers_longer_reset_time() {
        let mut target: Option<ModelLimitEntry> = None;
        update_min_entry(
            &mut target,
            ModelLimitEntry {
                remaining_fraction: 0.0,
                resets_at: None,
                reset_in_seconds: None,
            },
        );
        update_min_entry(
            &mut target,
            ModelLimitEntry {
                remaining_fraction: 0.0,
                resets_at: Some("2026-11-06 23:38:40".into()),
                reset_in_seconds: Some(18160),
            },
        );
        assert_eq!(target.as_ref().and_then(|e| e.reset_in_seconds), Some(18160));
    }
}
