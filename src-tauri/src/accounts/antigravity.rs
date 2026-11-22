use super::models::{AccountQuota, QuotaWindow};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

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

const CLOUD_CODE_DAILY_BASE_URL: &str = "https://daily-cloudcode-pa.googleapis.com";
const CLOUD_CODE_PROD_BASE_URL: &str = "https://cloudcode-pa.googleapis.com";
const CLOUD_CODE_AUTOPUSH_SANDBOX_BASE_URL: &str =
    "https://autopush-cloudcode-pa.sandbox.googleapis.com";
const LOAD_CODE_ASSIST_PATH: &str = "v1internal:loadCodeAssist";
const ONBOARD_USER_PATH: &str = "v1internal:onboardUser";
const FETCH_AVAILABLE_MODELS_PATH: &str = "v1internal:fetchAvailableModels";
const RETRIEVE_USER_QUOTA_SUMMARY_PATH: &str = "v1internal:retrieveUserQuotaSummary";

const DEFAULT_CLOUD_CODE_IDE_VERSION: &str = "2.25.0";
const DEFAULT_CLOUD_CODE_CL: &str = "1003475615";
const DEFAULT_LOAD_CODE_ASSIST_UA_OS: &str = "windows";
const DEFAULT_LOAD_CODE_ASSIST_UA_ARCH: &str = "amd64";
const DEFAULT_GOOGLE_API_NODEJS_CLIENT_VERSION: &str = "10.3.0";
const DEFAULT_X_GOOG_API_CLIENT_NODE_VERSION: &str = "22.21.1";
const ONBOARD_POLL_DELAY_MS: u64 = 500;
const ONBOARD_MAX_POLLS: usize = 15;

pub fn get_cli_token_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let gemini_cli = home
            .join(".gemini")
            .join("antigravity-cli")
            .join("antigravity-oauth-token");
        paths.push(gemini_cli);
        let jetski = home.join(".gemini").join("jetski-standalone-oauth-token");
        paths.push(jetski);
    }
    paths
}

pub fn read_current_credential_payload() -> Result<String, String> {
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
    #[cfg(target_os = "windows")]
    {
        super::crypto::cred_mgr::write_credential(
            "gemini:antigravity",
            "antigravity",
            payload_json.as_bytes(),
        )?;
    }

    for path in get_cli_token_paths() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, payload_json);
    }

    Ok(())
}

fn candidate_antigravity_profile_dirs(app_id: &str) -> Vec<PathBuf> {
    let mut dirs_list = Vec::new();
    let Some(config_dir) = dirs::config_dir() else {
        return dirs_list;
    };

    let subdirs: &[&str] = match app_id {
        "antigravity-app" => &["Antigravity"],
        "antigravity-ide" => &["Antigravity IDE", "AntigravityIDE"],
        _ => &["Antigravity", "Antigravity IDE", "AntigravityIDE"],
    };

    for sub in subdirs {
        let p = config_dir.join(sub);
        if p.is_dir() && !dirs_list.contains(&p) {
            dirs_list.push(p);
        }
    }

    dirs_list
}

pub(crate) fn clear_live_credentials_in_paths(
    token_paths: &[PathBuf],
    profile_dirs: &[PathBuf],
    clear_os_keyring: bool,
) -> Result<Vec<String>, String> {
    let mut cleared = Vec::new();
    let mut failures = Vec::new();

    #[cfg(target_os = "windows")]
    if clear_os_keyring {
        match super::crypto::cred_mgr::delete_credential_if_exists("gemini:antigravity") {
            Ok(true) => cleared.push("Windows Credential Manager: gemini:antigravity".to_string()),
            Ok(false) => {}
            Err(e) => failures.push(e),
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = clear_os_keyring;

    for path in token_paths {
        super::remove_file_strict(path, &mut cleared, &mut failures);
    }

    for profile_dir in profile_dirs {
        super::remove_chromium_cookie_files_strict(profile_dir, &mut cleared, &mut failures);
    }

    super::finalize_clear_result(cleared, failures)
}

pub fn clear_live_credentials(app_id: &str) -> Result<Vec<String>, String> {
    let token_paths = get_cli_token_paths();
    let profile_dirs = candidate_antigravity_profile_dirs(app_id);
    clear_live_credentials_in_paths(&token_paths, &profile_dirs, true)
}

pub fn parse_email_from_payload(payload_json: &str) -> Option<String> {
    let val: Value = serde_json::from_str(payload_json).ok()?;

    for key in &["id_token_email", "email", "user_email"] {
        if let Some(em) = val.get(key).and_then(|v| v.as_str()) {
            let trimmed = em.trim();
            if !trimmed.is_empty() && trimmed.contains('@') {
                return Some(trimmed.to_string());
            }
        }
    }

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
    let claims: Value = serde_json::from_slice(&decoded).ok()?;
    claims
        .get("email")
        .and_then(|e| e.as_str())
        .map(|s| s.to_string())
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

    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse refresh response: {}", e))?;

    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "No access_token in refresh response".to_string())?
        .to_string();

    let expires_in = json
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .unwrap_or(3600);

    Ok((access_token, expires_in))
}

pub async fn ensure_fresh_access_token(
    payload: &mut Value,
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

    let (new_access, expires_in) = refresh_access_token(&refresh_token, proxy).await?;
    let new_expiry = (Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339();

    token_obj["access_token"] = json!(new_access);
    token_obj["expiry"] = json!(new_expiry);

    Ok(new_access)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct QuotaCloudCodeContext {
    pub preferred_project_id: Option<String>,
    pub is_gcp_tos: bool,
}

impl QuotaCloudCodeContext {
    fn from_payload(val: &Value) -> Self {
        let project_id = val
            .get("project_id")
            .and_then(Value::as_str)
            .or_else(|| val.pointer("/token/project_id").and_then(Value::as_str));
        let is_gcp_tos = val
            .get("is_gcp_tos")
            .and_then(Value::as_bool)
            .or_else(|| val.pointer("/token/is_gcp_tos").and_then(Value::as_bool))
            .unwrap_or(false);
        Self {
            preferred_project_id: valid_cloud_code_project_id(project_id).map(str::to_string),
            is_gcp_tos,
        }
    }

    fn after_project_discovery(&self, metadata: &ProjectMetadataResult) -> Self {
        Self {
            preferred_project_id: valid_cloud_code_project_id(metadata.project_id.as_deref())
                .map(str::to_owned)
                .or_else(|| self.preferred_project_id.clone()),
            is_gcp_tos: metadata.is_gcp_tos.unwrap_or(self.is_gcp_tos),
        }
    }
}

fn valid_cloud_code_project_id(project_id: Option<&str>) -> Option<&str> {
    project_id
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "aicode-consumers")
}

fn env_var_trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_bool(name: &str) -> bool {
    matches!(
        env_var_trimmed(name)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn env_quality_is_insider_or_dev() -> bool {
    matches!(
        env_var_trimmed("ANTIGRAVITY_APP_QUALITY")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "insider" | "dev"
    )
}

pub(crate) fn select_cloud_code_base_url(
    ctx: &QuotaCloudCodeContext,
    override_url: Option<&str>,
    is_google_internal: bool,
    is_insider_or_dev: bool,
) -> String {
    // Preserve host routing order: override > GCP ToS with project > internal > daily.
    if let Some(override_url) = override_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return override_url.to_string();
    }

    if ctx.is_gcp_tos && valid_cloud_code_project_id(ctx.preferred_project_id.as_deref()).is_some()
    {
        return CLOUD_CODE_PROD_BASE_URL.to_string();
    }

    if is_google_internal && is_insider_or_dev {
        return CLOUD_CODE_AUTOPUSH_SANDBOX_BASE_URL.to_string();
    }

    CLOUD_CODE_DAILY_BASE_URL.to_string()
}

fn resolve_cloud_code_base_url(ctx: &QuotaCloudCodeContext) -> String {
    select_cloud_code_base_url(
        ctx,
        env_var_trimmed("ANTIGRAVITY_CLOUD_CODE_URL_OVERRIDE").as_deref(),
        env_bool("ANTIGRAVITY_IS_GOOGLE_INTERNAL"),
        env_quality_is_insider_or_dev(),
    )
}

fn parse_semver_tuple(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version
        .trim()
        .trim_start_matches('v')
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn resolve_cloud_code_version(installed_version: Option<&str>) -> String {
    let version = installed_version.unwrap_or("").trim();
    match (
        parse_semver_tuple(version),
        parse_semver_tuple(DEFAULT_CLOUD_CODE_IDE_VERSION),
    ) {
        (Some(detected), Some(minimum)) if detected >= minimum => version.to_string(),
        _ => DEFAULT_CLOUD_CODE_IDE_VERSION.to_string(),
    }
}

fn load_code_assist_user_agent_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "windows",
        "linux" => "linux",
        _ => DEFAULT_LOAD_CODE_ASSIST_UA_OS,
    }
}

fn load_code_assist_user_agent_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => DEFAULT_LOAD_CODE_ASSIST_UA_ARCH,
    }
}

fn cloud_code_platform_name() -> &'static str {
    match (
        load_code_assist_user_agent_os(),
        load_code_assist_user_agent_arch(),
    ) {
        ("darwin", "amd64") => "DARWIN_AMD64",
        ("darwin", "arm64") => "DARWIN_ARM64",
        ("linux", "amd64") => "LINUX_AMD64",
        ("linux", "arm64") => "LINUX_ARM64",
        ("windows", "amd64") => "WINDOWS_AMD64",
        _ => "PLATFORM_UNSPECIFIED",
    }
}

fn build_cloud_code_user_agent() -> String {
    let ide_version = resolve_cloud_code_version(None);
    let os = load_code_assist_user_agent_os();
    let arch = load_code_assist_user_agent_arch();
    format!(
        "antigravity/{} (aidev_client; os_type={}; arch={}; cl={})",
        ide_version, os, arch, DEFAULT_CLOUD_CODE_CL
    )
}

fn build_load_code_assist_user_agent() -> String {
    let ide_version = resolve_cloud_code_version(None);
    let os = load_code_assist_user_agent_os();
    let arch = load_code_assist_user_agent_arch();
    format!(
        "antigravity/{} {}/{} google-api-nodejs-client/{}",
        ide_version, os, arch, DEFAULT_GOOGLE_API_NODEJS_CLIENT_VERSION
    )
}

fn load_code_assist_x_goog_api_client() -> String {
    if let Some(trimmed) = env_var_trimmed("AG_LOAD_CODE_ASSIST_NODE_VERSION") {
        return format!("gl-node/{}", trimmed);
    }
    format!("gl-node/{}", DEFAULT_X_GOOG_API_CLIENT_NODE_VERSION)
}

fn build_cloud_code_metadata(duet_project: Option<&str>) -> Value {
    let mut metadata = serde_json::Map::new();
    metadata.insert("ideName".to_string(), Value::String("antigravity".to_string()));
    metadata.insert("ideType".to_string(), Value::String("ANTIGRAVITY".to_string()));
    metadata.insert(
        "ideVersion".to_string(),
        Value::String(resolve_cloud_code_version(None)),
    );
    metadata.insert(
        "pluginVersion".to_string(),
        Value::String(env!("CARGO_PKG_VERSION").to_string()),
    );
    metadata.insert(
        "platform".to_string(),
        Value::String(cloud_code_platform_name().to_string()),
    );
    metadata.insert("updateChannel".to_string(), Value::String("stable".to_string()));
    metadata.insert("pluginType".to_string(), Value::String("GEMINI".to_string()));
    if let Some(project) = valid_cloud_code_project_id(duet_project) {
        metadata.insert("duetProject".to_string(), Value::String(project.to_string()));
    }
    Value::Object(metadata)
}

fn build_load_code_assist_payload(project_id: Option<&str>) -> Value {
    let mut payload = json!({
        "metadata": build_cloud_code_metadata(project_id),
        "mode": "FULL_ELIGIBILITY_CHECK"
    });
    if let Some(pid) = valid_cloud_code_project_id(project_id) {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(
                "cloudaicompanionProject".to_string(),
                Value::String(pid.to_string()),
            );
        }
    }
    payload
}

#[derive(Debug, Deserialize)]
struct LoadProjectResponse {
    #[serde(rename = "cloudaicompanionProject")]
    project: Option<Value>,
    #[serde(rename = "currentTier")]
    current_tier: Option<Tier>,
    #[serde(rename = "paidTier")]
    paid_tier: Option<Tier>,
    #[serde(rename = "allowedTiers")]
    allowed_tiers: Option<Vec<AllowedTier>>,
}

impl LoadProjectResponse {
    fn current_gcp_tos(&self) -> Option<bool> {
        let current = self.current_tier.as_ref()?;
        current.uses_gcp_tos.or_else(|| {
            let id = current.id.as_deref()?;
            self.allowed_tiers.as_ref()?.iter().find_map(|tier| {
                (tier.id.as_deref() == Some(id))
                    .then_some(tier.uses_gcp_tos)
                    .flatten()
            })
        })
    }
}

#[derive(Debug, Deserialize)]
struct AllowedTier {
    id: Option<String>,
    #[serde(rename = "isDefault")]
    is_default: Option<bool>,
    #[serde(rename = "usesGcpTos")]
    uses_gcp_tos: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct Tier {
    id: Option<String>,
    name: Option<String>,
    #[serde(rename = "usesGcpTos")]
    uses_gcp_tos: Option<bool>,
    #[serde(rename = "availableCredits", default)]
    available_credits: Option<Vec<AvailableCreditRaw>>,
}

#[derive(Debug, Deserialize)]
struct AvailableCreditRaw {
    #[serde(rename = "creditType")]
    credit_type: Option<String>,
    #[serde(rename = "creditAmount")]
    credit_amount: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OnboardUserResponse {
    name: Option<String>,
    done: Option<bool>,
    response: Option<OnboardResponse>,
}

#[derive(Debug, Deserialize)]
struct OnboardResponse {
    #[serde(rename = "cloudaicompanionProject")]
    project: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectMetadataResult {
    pub loaded_ok: bool,
    pub project_id: Option<String>,
    pub tier_id: Option<String>,
    pub tier_display: Option<String>,
    pub total_ai_credits: Option<String>,
    pub is_gcp_tos: Option<bool>,
}

fn extract_project_id(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return valid_cloud_code_project_id(Some(text)).map(str::to_string);
    }
    if let Some(id) = value.get("id").and_then(Value::as_str) {
        return valid_cloud_code_project_id(Some(id)).map(str::to_string);
    }
    None
}

fn pick_onboard_tier(allowed: &[AllowedTier]) -> Option<String> {
    if let Some(id) = allowed
        .iter()
        .find(|tier| tier.is_default.unwrap_or(false))
        .and_then(|tier| tier.id.clone())
    {
        return Some(id);
    }
    if let Some(id) = allowed.iter().find_map(|tier| tier.id.clone()) {
        return Some(id);
    }
    if !allowed.is_empty() {
        return Some("LEGACY".to_string());
    }
    None
}

fn extract_total_ai_credits(tier: &Tier) -> Option<String> {
    let credits = tier.available_credits.as_ref()?;
    let mut total = 0.0_f64;
    let mut has_valid_amount = false;
    for item in credits {
        if item.credit_type.is_none() {
            continue;
        }
        let Some(raw) = item.credit_amount.as_deref() else {
            continue;
        };
        let cleaned = raw.replace(',', "").trim().to_string();
        if let Ok(parsed) = cleaned.parse::<f64>() {
            if parsed.is_finite() {
                total += parsed;
                has_valid_amount = true;
            }
        }
    }
    if !has_valid_amount {
        return None;
    }
    let formatted = format!("{:.2}", total);
    let trimmed = formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string();
    Some(trimmed)
}

/// Normalize raw tier identifier (`g1-ultra-tier`, `g1-pro-tier`, `free-tier`, etc.) to `ULTRA` / `PRO` / `FREE`.
pub(crate) fn normalize_subscription_tier_code(raw: Option<&str>) -> Option<&'static str> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("ultra") {
        Some("ULTRA")
    } else if lower.contains("pro") {
        Some("PRO")
    } else {
        Some("FREE")
    }
}

fn format_plan_type_label(tier_code: &str) -> String {
    match tier_code {
        "ULTRA" => "Ultra".to_string(),
        "PRO" => "Pro".to_string(),
        "FREE" => "Free".to_string(),
        other => other.to_string(),
    }
}

async fn try_onboard_user(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    tier_id: &str,
    project_id: Option<&str>,
) -> Result<Option<String>, String> {
    let mut payload = json!({
        "tierId": tier_id,
        "metadata": build_cloud_code_metadata(project_id)
    });
    let ua = build_load_code_assist_user_agent();
    if let Some(pid) = valid_cloud_code_project_id(project_id) {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(
                "cloudaicompanionProject".to_string(),
                Value::String(pid.to_string()),
            );
        }
    }

    let response = client
        .post(format!("{}/{}", base_url, ONBOARD_USER_PATH))
        .bearer_auth(access_token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, &ua)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("onboardUser failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("onboardUser returned {} - {}", status, text));
    }

    let mut data = response
        .json::<OnboardUserResponse>()
        .await
        .map_err(|e| format!("Failed to parse onboardUser response: {}", e))?;

    for _ in 0..ONBOARD_MAX_POLLS {
        if data.done.unwrap_or(false) {
            return Ok(data
                .response
                .and_then(|resp| resp.project)
                .and_then(|proj| extract_project_id(&proj)));
        }

        let Some(op_name) = data
            .name
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        else {
            return Ok(None);
        };

        tokio::time::sleep(std::time::Duration::from_millis(ONBOARD_POLL_DELAY_MS)).await;

        let poll_response = client
            .get(format!("{}/v1internal/{}", base_url, op_name))
            .bearer_auth(access_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::USER_AGENT, &ua)
            .send()
            .await
            .map_err(|e| format!("onboardUser poll failed: {}", e))?;

        if !poll_response.status().is_success() {
            let status = poll_response.status();
            let text = poll_response.text().await.unwrap_or_default();
            return Err(format!("onboardUser poll returned {} - {}", status, text));
        }

        data = poll_response
            .json::<OnboardUserResponse>()
            .await
            .map_err(|e| format!("Failed to parse onboardUser poll response: {}", e))?;
    }

    Ok(None)
}

async fn fetch_project_metadata_on_base_url(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    preferred_project_id: Option<&str>,
) -> Option<ProjectMetadataResult> {
    let ua = build_load_code_assist_user_agent();
    let x_goog_api_client = load_code_assist_x_goog_api_client();

    let res = match client
        .post(format!("{}/{}", base_url, LOAD_CODE_ASSIST_PATH))
        .bearer_auth(access_token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, &ua)
        .header("x-goog-api-client", &x_goog_api_client)
        .header(reqwest::header::ACCEPT, "*/*")
        .json(&build_load_code_assist_payload(preferred_project_id))
        .send()
        .await
    {
        Ok(res) => res,
        Err(err) => {
            eprintln!("[Antigravity] loadCodeAssist request to {} failed: {}", base_url, err);
            return None;
        }
    };

    if !res.status().is_success() {
        eprintln!(
            "[Antigravity] loadCodeAssist on {} returned HTTP {}",
            base_url,
            res.status()
        );
        return None;
    }

    let data = match res.json::<LoadProjectResponse>().await {
        Ok(data) => data,
        Err(err) => {
            eprintln!("[Antigravity] Failed to parse loadCodeAssist response: {}", err);
            return None;
        }
    };

    let paid_tier_id = data
        .paid_tier
        .as_ref()
        .and_then(|t| t.id.clone().or_else(|| t.name.clone()));
    let current_tier_id = data
        .current_tier
        .as_ref()
        .and_then(|t| t.id.clone().or_else(|| t.name.clone()));
    let tier_id = paid_tier_id.or(current_tier_id);
    let tier_display =
        normalize_subscription_tier_code(tier_id.as_deref()).map(format_plan_type_label);
    // If loadCodeAssist succeeded without GCP ToS evidence, reset to false so stale true is cleared.
    let is_gcp_tos = Some(
        data.current_gcp_tos()
            .or_else(|| data.paid_tier.as_ref().and_then(|t| t.uses_gcp_tos))
            .unwrap_or(false),
    );
    let total_ai_credits = data.paid_tier.as_ref().and_then(extract_total_ai_credits);

    let response_project_id = data.project.as_ref().and_then(extract_project_id);
    if let Some(pid) = response_project_id {
        return Some(ProjectMetadataResult {
            loaded_ok: true,
            project_id: Some(pid),
            tier_id,
            tier_display,
            total_ai_credits,
            is_gcp_tos,
        });
    }

    let allowed_tiers = data.allowed_tiers.unwrap_or_default();
    let onboard_tier = pick_onboard_tier(&allowed_tiers).or_else(|| tier_id.clone());
    let mut onboarded_project_id = None;
    if let Some(onboard_tier_id) = onboard_tier {
        match try_onboard_user(
            client,
            base_url,
            access_token,
            &onboard_tier_id,
            preferred_project_id,
        )
        .await
        {
            Ok(pid) => onboarded_project_id = pid,
            Err(err) => {
                eprintln!("[Antigravity] onboardUser on {} failed: {}", base_url, err);
            }
        }
    }

    Some(ProjectMetadataResult {
        loaded_ok: true,
        project_id: onboarded_project_id,
        tier_id,
        tier_display,
        total_ai_credits,
        is_gcp_tos,
    })
}

async fn fetch_project_metadata(
    client: &reqwest::Client,
    access_token: &str,
    ctx: &QuotaCloudCodeContext,
) -> ProjectMetadataResult {
    let base_url = resolve_cloud_code_base_url(ctx);
    let preferred_project_id = valid_cloud_code_project_id(ctx.preferred_project_id.as_deref());

    if let Some(meta) =
        fetch_project_metadata_on_base_url(client, &base_url, access_token, preferred_project_id)
            .await
    {
        return meta;
    }

    // Prevent routing deadlock if cached is_gcp_tos sent us to PROD and the account reverted to consumer.
    if base_url == CLOUD_CODE_PROD_BASE_URL {
        if let Some(meta) = fetch_project_metadata_on_base_url(
            client,
            CLOUD_CODE_DAILY_BASE_URL,
            access_token,
            preferred_project_id,
        )
        .await
        {
            return meta;
        }
    }

    ProjectMetadataResult::default()
}

#[derive(Debug, Clone)]
pub(crate) struct QuotaFetchError {
    pub code: Option<u16>,
    pub message: String,
    pub reason: Option<String>,
    pub validation_url: Option<String>,
}

pub(crate) fn is_explicit_quota_denial(
    reason: Option<&str>,
    validation_url: Option<&str>,
) -> bool {
    matches!(reason, Some("VALIDATION_REQUIRED" | "SUBSCRIPTION_REQUIRED"))
        || validation_url.is_some_and(|url| !url.trim().is_empty())
}

pub(crate) fn format_google_validation_url(raw_url: &str, email: &str) -> String {
    let email = email.trim();
    if raw_url.trim().is_empty() || email.is_empty() {
        return raw_url.to_string();
    }

    let Some(scheme_end) = raw_url.find("://") else {
        return raw_url.to_string();
    };
    let scheme = &raw_url[..scheme_end];
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return raw_url.to_string();
    }
    let after_scheme = &raw_url[scheme_end + 3..];
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    let host_no_user = authority.rsplit('@').next().unwrap_or(authority);
    let host = host_no_user
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_google_host =
        host == "accounts.google.com" || host == "google.com" || host.ends_with(".google.com");
    if !is_google_host {
        return raw_url.to_string();
    }

    let rest = &after_scheme[authority_end..];
    let (before_frag, fragment) = match rest.split_once('#') {
        Some((before, frag)) => (before, Some(frag)),
        None => (rest, None),
    };
    let (path, query) = match before_frag.split_once('?') {
        Some((p, q)) => (p, q),
        None => (before_frag, ""),
    };

    let mut pairs: Vec<(String, String)> = Vec::new();
    if !query.is_empty() {
        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let mut parts = pair.splitn(2, '=');
            let k = super::oauth::url_decode(parts.next().unwrap_or(""));
            let v = super::oauth::url_decode(parts.next().unwrap_or(""));
            if !k.is_empty() {
                pairs.push((k, v));
            }
        }
    }

    let has_valid_authuser = pairs.iter().any(|(k, v)| {
        k == "authuser" && !v.trim().is_empty() && !v.trim().chars().all(|c| c.is_ascii_digit())
    });
    let has_login_hint = pairs
        .iter()
        .any(|(k, v)| k == "login_hint" && !v.trim().is_empty());
    let has_email = pairs
        .iter()
        .any(|(k, v)| k == "Email" && !v.trim().is_empty());

    if !has_valid_authuser {
        pairs.retain(|(k, _)| k != "authuser");
        pairs.push(("authuser".to_string(), email.to_string()));
    }
    if !has_login_hint {
        pairs.retain(|(k, _)| k != "login_hint");
        pairs.push(("login_hint".to_string(), email.to_string()));
    }
    if !has_email {
        pairs.retain(|(k, _)| k != "Email");
        pairs.push(("Email".to_string(), email.to_string()));
    }

    let encoded_query = pairs
        .into_iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                super::oauth::url_encode(&k),
                super::oauth::url_encode(&v)
            )
        })
        .collect::<Vec<_>>()
        .join("&");

    let mut out = format!("{}://{}{}", scheme, authority, path);
    if !encoded_query.is_empty() {
        out.push('?');
        out.push_str(&encoded_query);
    }
    if let Some(frag) = fragment {
        out.push('#');
        out.push_str(frag);
    }
    out
}

fn parse_google_api_error_with_email(
    status: u16,
    text: &str,
    email: Option<&str>,
) -> QuotaFetchError {
    let mut message = if text.trim().is_empty() {
        format!("API returned status {}", status)
    } else {
        text.to_string()
    };
    let mut reason: Option<String> = None;
    let mut validation_url: Option<String> = None;

    if let Ok(val) = serde_json::from_str::<Value>(text) {
        if let Some(err_obj) = val.get("error") {
            if let Some(msg) = err_obj.get("message").and_then(|v| v.as_str()) {
                message = msg.to_string();
            }
            if let Some(details) = err_obj.get("details").and_then(|v| v.as_array()) {
                for detail in details {
                    if reason.is_none() {
                        if let Some(r) = detail.get("reason").and_then(|v| v.as_str()) {
                            reason = Some(r.to_string());
                        }
                    }
                    if validation_url.is_none() {
                        if let Some(url) = detail
                            .pointer("/metadata/validation_url")
                            .and_then(|v| v.as_str())
                        {
                            validation_url = Some(url.to_string());
                        }
                    }
                }
            }
        }
    }

    if reason.is_none() {
        if message.contains("Verify your account") {
            reason = Some("VALIDATION_REQUIRED".to_string());
        } else if message.contains("Subscription required") {
            reason = Some("SUBSCRIPTION_REQUIRED".to_string());
        }
    }

    if let (Some(url), Some(em)) = (validation_url.as_ref(), email) {
        validation_url = Some(format_google_validation_url(url, em));
    }

    QuotaFetchError {
        code: Some(status),
        message,
        reason,
        validation_url,
    }
}

#[derive(Debug, Deserialize)]
struct QuotaResponse {
    #[serde(default)]
    models: HashMap<String, ModelInfo>,
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
/// 当 `remainingFraction` 为 `None` 且存在 `resetTime` 时，说明额度已耗尽（0.0）。
fn resolve_remaining_fraction(remaining_fraction: Option<f64>, reset_time: Option<&str>) -> f64 {
    match remaining_fraction {
        Some(v) => v.clamp(0.0, 1.0),
        None if reset_time.is_some_and(|rt| !rt.trim().is_empty()) => 0.0,
        None => 1.0,
    }
}

fn round_quota_percent(remaining_fraction: f64) -> (f64, f64) {
    let rem = (remaining_fraction * 100.0).round().clamp(0.0, 100.0);
    let used = (100.0 - rem).clamp(0.0, 100.0);
    (rem, used)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryBucketFamily {
    Claude,
    Gemini,
    Argon,
    Carbon,
}

fn canonical_bucket_key(bucket_id: &str) -> Option<&'static str> {
    let lower = bucket_id.trim().to_ascii_lowercase();
    match lower.as_str() {
        "3p-5h" | "claude:5h" => Some("claude:5h"),
        "3p-weekly" | "claude:weekly" => Some("claude:weekly"),
        "gemini-5h" | "gemini:5h" => Some("gemini:5h"),
        "gemini-weekly" | "gemini:weekly" => Some("gemini:weekly"),
        "argon:weekly" | "gemini-argon-wtu-weekly" => Some("argon:weekly"),
        "carbon:weekly" | "gemini-carbon-wtu-weekly" => Some("carbon:weekly"),
        name if name.starts_with("gemini-argon-") && name.ends_with("-weekly") => {
            Some("argon:weekly")
        }
        name if name.starts_with("gemini-carbon-") && name.ends_with("-weekly") => {
            Some("carbon:weekly")
        }
        _ => None,
    }
}

fn is_ignored_short_bucket(bucket_id: &str) -> bool {
    let lower = bucket_id.trim().to_ascii_lowercase();
    matches!(lower.as_str(), "argon:5h" | "gemini-argon-5h")
        || (lower.starts_with("gemini-argon-") && lower.ends_with("-5h"))
        || matches!(lower.as_str(), "carbon:5h" | "gemini-carbon-5h")
        || (lower.starts_with("gemini-carbon-") && lower.ends_with("-5h"))
}

fn summary_bucket_family(key_or_name: &str) -> Option<SummaryBucketFamily> {
    let lower = key_or_name.trim().to_ascii_lowercase();
    match lower.as_str() {
        "3p-5h" | "claude:5h" | "3p-weekly" | "claude:weekly" => Some(SummaryBucketFamily::Claude),
        "gemini-5h" | "gemini:5h" | "gemini-weekly" | "gemini:weekly" => {
            Some(SummaryBucketFamily::Gemini)
        }
        "argon:weekly" | "argon:5h" | "gemini-argon-wtu-weekly" => Some(SummaryBucketFamily::Argon),
        "carbon:weekly" | "gemini-carbon-wtu-weekly" => Some(SummaryBucketFamily::Carbon),
        name if name.starts_with("gemini-argon-")
            && (name.ends_with("-weekly") || name.ends_with("-5h")) =>
        {
            Some(SummaryBucketFamily::Argon)
        }
        name if name.starts_with("gemini-carbon-")
            && (name.ends_with("-weekly") || name.ends_with("-5h")) =>
        {
            Some(SummaryBucketFamily::Carbon)
        }
        _ => None,
    }
}

fn default_bucket_label(canonical_key: &str) -> String {
    match canonical_key {
        "gemini:5h" => "Gemini · 5h".to_string(),
        "gemini:weekly" => "Gemini · Weekly".to_string(),
        "claude:5h" => "Claude · 5h".to_string(),
        "claude:weekly" => "Claude · Weekly".to_string(),
        "argon:weekly" => "Gemini 4 Argon · Weekly".to_string(),
        "carbon:weekly" => "Gemini 4 Carbon · Weekly".to_string(),
        other => other.to_string(),
    }
}

fn default_bucket_detail(canonical_key: &str) -> Option<String> {
    match canonical_key {
        "gemini:5h" => Some("Gemini 5小时速率窗口".to_string()),
        "gemini:weekly" => Some("Gemini 每周总用量池".to_string()),
        "claude:5h" => Some("Claude & GPT 5小时速率窗口".to_string()),
        "claude:weekly" => Some("Claude & GPT 每周总用量池".to_string()),
        "argon:weekly" => Some("Gemini 4 Argon 独立周总用量池".to_string()),
        "carbon:weekly" => Some("Gemini 4 Carbon 独立周总用量池".to_string()),
        _ => None,
    }
}

struct CanonicalModelSpec {
    id: &'static str,
    display_name: &'static str,
    model_constant: Option<&'static str>,
    aliases: &'static [&'static str],
}

const CANONICAL_MODELS: &[CanonicalModelSpec] = &[
    CanonicalModelSpec {
        id: "gemini-4-carbon",
        display_name: "Gemini 4 Carbon",
        model_constant: None,
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-carbon-medium",
        display_name: "Gemini 4 Carbon (Medium)",
        model_constant: None,
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-carbon-high",
        display_name: "Gemini 4 Carbon (High)",
        model_constant: None,
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-carbon-low",
        display_name: "Gemini 4 Carbon (Low)",
        model_constant: None,
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-argon",
        display_name: "Gemini 4 Argon",
        model_constant: None,
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-argon-medium",
        display_name: "Gemini 4 Argon (Medium)",
        model_constant: Some("MODEL_PLACEHOLDER_M418"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-argon-high",
        display_name: "Gemini 4 Argon (High)",
        model_constant: Some("MODEL_PLACEHOLDER_M417"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-argon-low",
        display_name: "Gemini 4 Argon (Low)",
        model_constant: Some("MODEL_PLACEHOLDER_M419"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-hydrogen",
        display_name: "Gemini 4 Hydrogen",
        model_constant: None,
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-hydrogen-low",
        display_name: "Gemini 4 Hydrogen (Low)",
        model_constant: Some("MODEL_PLACEHOLDER_M527"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-hydrogen-medium",
        display_name: "Gemini 4 Hydrogen (Medium)",
        model_constant: Some("MODEL_PLACEHOLDER_M526"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-hydrogen-high",
        display_name: "Gemini 4 Hydrogen (High)",
        model_constant: Some("MODEL_PLACEHOLDER_M525"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-4-hydrogen-xhigh",
        display_name: "Gemini 4 Hydrogen (XHigh)",
        model_constant: Some("MODEL_PLACEHOLDER_M528"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-3.8-flash",
        display_name: "Gemini 3.8 Flash",
        model_constant: None,
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-3.8-flash-low",
        display_name: "Gemini 3.8 Flash (Low)",
        model_constant: Some("MODEL_PLACEHOLDER_M320"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-3.8-flash-medium",
        display_name: "Gemini 3.8 Flash (Medium)",
        model_constant: Some("MODEL_PLACEHOLDER_M319"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-3.8-flash-high",
        display_name: "Gemini 3.8 Flash (High)",
        model_constant: Some("MODEL_PLACEHOLDER_M318"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "claude-opus-5-5",
        display_name: "Claude Opus 5.5",
        model_constant: None,
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "claude-opus-5-5-low",
        display_name: "Claude Opus 5.5 (Low)",
        model_constant: Some("MODEL_PLACEHOLDER_M400"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "claude-opus-5-5-medium",
        display_name: "Claude Opus 5.5 (Medium)",
        model_constant: Some("MODEL_PLACEHOLDER_M401"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "claude-opus-5-5-high",
        display_name: "Claude Opus 5.5 (High)",
        model_constant: Some("MODEL_PLACEHOLDER_M402"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "claude-sonnet-5-5",
        display_name: "Claude Sonnet 5.5",
        model_constant: None,
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "claude-sonnet-5-5-low",
        display_name: "Claude Sonnet 5.5 (Low)",
        model_constant: Some("MODEL_PLACEHOLDER_M403"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "claude-sonnet-5-5-medium",
        display_name: "Claude Sonnet 5.5 (Medium)",
        model_constant: Some("MODEL_PLACEHOLDER_M404"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "claude-sonnet-5-5-high",
        display_name: "Claude Sonnet 5.5 (High)",
        model_constant: Some("MODEL_PLACEHOLDER_M405"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "gemini-3.1-pro-high",
        display_name: "Gemini 3.1 Pro (High)",
        model_constant: Some("MODEL_PLACEHOLDER_M37"),
        aliases: &["gemini-3-pro-high", "MODEL_PLACEHOLDER_M8"],
    },
    CanonicalModelSpec {
        id: "gemini-3.1-pro-low",
        display_name: "Gemini 3.1 Pro (Low)",
        model_constant: Some("MODEL_PLACEHOLDER_M36"),
        aliases: &["gemini-3-pro-low", "MODEL_PLACEHOLDER_M7"],
    },
    CanonicalModelSpec {
        id: "gemini-3-flash",
        display_name: "Gemini 3 Flash",
        model_constant: Some("MODEL_PLACEHOLDER_M18"),
        aliases: &[],
    },
    CanonicalModelSpec {
        id: "claude-sonnet-4-6",
        display_name: "Claude Sonnet 4.6 (Thinking)",
        model_constant: Some("MODEL_PLACEHOLDER_M35"),
        aliases: &[
            "claude-sonnet-4-6-thinking",
            "claude-sonnet-4-5",
            "claude-sonnet-4-5-thinking",
        ],
    },
    CanonicalModelSpec {
        id: "claude-opus-4-6-thinking",
        display_name: "Claude Opus 4.6 (Thinking)",
        model_constant: Some("MODEL_PLACEHOLDER_M26"),
        aliases: &[
            "claude-opus-4-6",
            "claude-opus-4-5-thinking",
            "MODEL_PLACEHOLDER_M12",
        ],
    },
    CanonicalModelSpec {
        id: "gpt-oss-120b-medium",
        display_name: "GPT-OSS 120B (Medium)",
        model_constant: Some("MODEL_OPENAI_GPT_OSS_120B_MEDIUM"),
        aliases: &[],
    },
];

fn normalize_model_identity(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

fn canonical_model_index() -> &'static HashMap<String, usize> {
    static INDEX: OnceLock<HashMap<String, usize>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut map = HashMap::new();
        for (idx, spec) in CANONICAL_MODELS.iter().enumerate() {
            for key in [Some(spec.id), Some(spec.display_name), spec.model_constant]
                .into_iter()
                .flatten()
                .chain(spec.aliases.iter().copied())
            {
                let norm = normalize_model_identity(key);
                if !norm.is_empty() {
                    map.entry(norm).or_insert(idx);
                }
                if let Some(stripped) = key.strip_prefix("MODEL_") {
                    let norm_stripped = normalize_model_identity(stripped);
                    if !norm_stripped.is_empty() {
                        map.entry(norm_stripped).or_insert(idx);
                    }
                }
            }
        }
        map
    })
}

fn resolve_canonical_model(
    key: &str,
    display_name: Option<&str>,
) -> Option<(usize, &'static CanonicalModelSpec)> {
    let index = canonical_model_index();
    for raw in [Some(key), display_name].into_iter().flatten() {
        let norm = normalize_model_identity(raw);
        if norm.is_empty() {
            continue;
        }
        if let Some(&idx) = index.get(&norm) {
            return Some((idx, &CANONICAL_MODELS[idx]));
        }
    }
    None
}

fn is_summary_payload_valid(summary: &Value) -> bool {
    let Some(groups) = summary.get("groups").and_then(Value::as_array) else {
        return false;
    };
    groups.iter().all(|group| {
        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            // Proto3 omits empty repeated buckets on groups without active limits.
            return group.get("buckets").is_none();
        };
        buckets.iter().all(|bucket| {
            let has_id = bucket
                .get("bucketId")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.trim().is_empty());
            let has_fraction = bucket
                .get("remainingFraction")
                .and_then(Value::as_f64)
                .is_some();
            let has_reset = bucket
                .get("resetTime")
                .and_then(Value::as_str)
                .is_some_and(|rt| !rt.trim().is_empty());
            has_id && (has_fraction || has_reset)
        })
    })
}

struct BucketAccumulator {
    raw_fraction: f64,
    window: QuotaWindow,
}

fn upsert_bucket_entry(buckets: &mut Vec<BucketAccumulator>, candidate: BucketAccumulator) {
    let cand_key = candidate
        .window
        .window_key
        .as_deref()
        .unwrap_or(&candidate.window.name);
    if let Some(existing) = buckets.iter_mut().find(|entry| {
        entry
            .window
            .window_key
            .as_deref()
            .unwrap_or(&entry.window.name)
            == cand_key
    }) {
        let should_replace = if candidate.raw_fraction < existing.raw_fraction - f64::EPSILON {
            true
        } else if (candidate.raw_fraction - existing.raw_fraction).abs() <= f64::EPSILON {
            candidate.window.reset_in_seconds.unwrap_or(-1)
                > existing.window.reset_in_seconds.unwrap_or(-1)
        } else {
            false
        };
        if should_replace {
            *existing = candidate;
        }
    } else {
        buckets.push(candidate);
    }
}

fn parse_summary_buckets(
    summary: Option<&Value>,
    is_free: bool,
    now: DateTime<Utc>,
) -> Vec<QuotaWindow> {
    let mut accumulated = Vec::new();
    let Some(groups) = summary.and_then(|s| s.get("groups")).and_then(Value::as_array) else {
        return Vec::new();
    };

    for group in groups {
        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            continue;
        };
        for bucket in buckets {
            let Some(raw_bucket_id) = bucket
                .get("bucketId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };

            if is_ignored_short_bucket(raw_bucket_id) {
                continue;
            }

            let raw_reset = bucket
                .get("resetTime")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let raw_fraction = bucket.get("remainingFraction").and_then(Value::as_f64);
            if raw_fraction.is_none() && raw_reset.is_none() {
                continue;
            }

            let canonical_key = canonical_bucket_key(raw_bucket_id)
                .map(str::to_string)
                .unwrap_or_else(|| raw_bucket_id.to_string());

            if is_free && canonical_key.ends_with(":5h") {
                continue;
            }

            let rem_frac = resolve_remaining_fraction(raw_fraction, raw_reset.as_deref());
            let (rem_pct, used_pct) = round_quota_percent(rem_frac);
            let (resets_at, reset_sec) = parse_reset_time(raw_reset.as_deref(), now);
            let summary_exhausted = Some(rem_frac == 0.0);

            let display_name = bucket
                .get("displayName")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| default_bucket_label(&canonical_key));

            upsert_bucket_entry(
                &mut accumulated,
                BucketAccumulator {
                    raw_fraction: rem_frac,
                    window: QuotaWindow {
                        name: display_name,
                        used_percent: Some(used_pct),
                        remaining_percent: Some(rem_pct),
                        resets_at,
                        reset_in_seconds: reset_sec,
                        detail: default_bucket_detail(&canonical_key)
                            .or_else(|| Some(format!("Bucket ID: {}", raw_bucket_id))),
                        window_key: Some(canonical_key),
                        window_kind: Some("bucket".to_string()),
                        stale: None,
                        blocked_by_weekly: None,
                        summary_exhausted,
                        reset_time_iso: raw_reset,
                    },
                },
            );
        }
    }

    accumulated.into_iter().map(|entry| entry.window).collect()
}

fn parse_model_windows(quota_response: &QuotaResponse, now: DateTime<Utc>) -> Vec<QuotaWindow> {
    let mut canonical_picked: Vec<(usize, QuotaWindow)> = Vec::new();
    let mut seen_canonical_ids: HashSet<&'static str> = HashSet::new();
    let mut extra_models: Vec<(String, QuotaWindow)> = Vec::new();

    let mut sorted_keys: Vec<_> = quota_response.models.keys().cloned().collect();
    sorted_keys.sort();

    for key in sorted_keys {
        let key_lower = key.to_ascii_lowercase();
        if key_lower.starts_with("chat_") || key_lower.starts_with("tab_") {
            continue;
        }
        if canonical_bucket_key(&key).is_some() || is_ignored_short_bucket(&key) {
            continue;
        }

        let info = &quota_response.models[&key];
        let Some(q) = &info.quota_info else {
            continue;
        };

        let api_display = info
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let raw_reset = q
            .reset_time
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let rem_frac = resolve_remaining_fraction(q.remaining_fraction, raw_reset.as_deref());
        let (rem_pct, used_pct) = round_quota_percent(rem_frac);
        let (resets_at, reset_sec) = parse_reset_time(raw_reset.as_deref(), now);

        if let Some((rank, spec)) = resolve_canonical_model(&key, api_display) {
            if !seen_canonical_ids.insert(spec.id) {
                continue;
            }
            let name = api_display.unwrap_or(spec.display_name).to_string();
            canonical_picked.push((
                rank,
                QuotaWindow {
                    name,
                    used_percent: Some(used_pct),
                    remaining_percent: Some(rem_pct),
                    resets_at,
                    reset_in_seconds: reset_sec,
                    detail: Some(format!("Model ID: {}", key)),
                    window_key: Some(spec.id.to_string()),
                    window_kind: Some("model".to_string()),
                    stale: None,
                    blocked_by_weekly: None,
                    summary_exhausted: None,
                    reset_time_iso: raw_reset,
                },
            ));
        } else {
            let name = api_display.unwrap_or(&key).to_string();
            extra_models.push((
                key.clone(),
                QuotaWindow {
                    name,
                    used_percent: Some(used_pct),
                    remaining_percent: Some(rem_pct),
                    resets_at,
                    reset_in_seconds: reset_sec,
                    detail: Some(format!("Model ID: {}", key)),
                    window_key: Some(key),
                    window_kind: Some("model".to_string()),
                    stale: None,
                    blocked_by_weekly: None,
                    summary_exhausted: None,
                    reset_time_iso: raw_reset,
                },
            ));
        }
    }

    canonical_picked.sort_by_key(|(rank, _)| *rank);
    extra_models.sort_by(|(a, _), (b, _)| a.cmp(b));

    let mut out: Vec<QuotaWindow> = canonical_picked.into_iter().map(|(_, w)| w).collect();
    out.extend(extra_models.into_iter().map(|(_, w)| w));
    out
}

fn bucket_order_rank(key: &str) -> usize {
    match key {
        "gemini:5h" => 0,
        "gemini:weekly" => 1,
        "claude:5h" => 2,
        "claude:weekly" => 3,
        "argon:weekly" => 4,
        "carbon:weekly" => 5,
        _ => 100,
    }
}

fn is_bucket_window(w: &QuotaWindow) -> bool {
    if w.window_kind.as_deref() == Some("bucket") {
        return true;
    }
    if w.window_kind.is_none() {
        return w
            .window_key
            .as_deref()
            .is_some_and(|k| summary_bucket_family(k).is_some())
            || w.name.contains("周用量限制")
            || w.name.contains("5小时速率");
    }
    false
}

fn legacy_window_to_canonical_key(w: &QuotaWindow) -> Option<&'static str> {
    if let Some(k) = w.window_key.as_deref().and_then(canonical_bucket_key) {
        return Some(k);
    }
    let name_lower = w.name.to_ascii_lowercase();
    if name_lower.contains("argon") && w.name.contains("周") {
        Some("argon:weekly")
    } else if name_lower.contains("carbon") && w.name.contains("周") {
        Some("carbon:weekly")
    } else if name_lower.contains("gemini") && w.name.contains("周") {
        Some("gemini:weekly")
    } else if name_lower.contains("gemini") && w.name.contains("5") {
        Some("gemini:5h")
    } else if (name_lower.contains("claude") || name_lower.contains("gpt")) && w.name.contains("周")
    {
        Some("claude:weekly")
    } else if (name_lower.contains("claude") || name_lower.contains("gpt")) && w.name.contains("5")
    {
        Some("claude:5h")
    } else {
        None
    }
}

fn is_reauth_error(err: &QuotaFetchError) -> bool {
    err.reason.as_deref() == Some("VALIDATION_REQUIRED")
        || err.validation_url.as_deref().is_some_and(|u| !u.trim().is_empty())
        || err.message.contains("Verify your account")
}

fn preserve_failed_quota_summary(
    buckets: &mut Vec<QuotaWindow>,
    quota_summary_stale: bool,
    is_forbidden: bool,
    is_reauth: bool,
    current_project_id: Option<&str>,
    current_tier_id: Option<&str>,
    previous: Option<&AccountQuota>,
) {
    if !quota_summary_stale || (is_forbidden && !is_reauth) {
        return;
    }
    let Some(prev) = previous else {
        return;
    };
    let prev_forbidden = prev.extra_info.get("is_forbidden").map(String::as_str) == Some("true");
    let prev_reauth = prev
        .extra_info
        .get("quota_error_reason")
        .is_some_and(|r| r == "VALIDATION_REQUIRED")
        || prev.extra_info.contains_key("validation_url");
    if prev_forbidden && !prev_reauth {
        return;
    }
    let prev_project = prev.extra_info.get("project_id").map(String::as_str);
    let prev_tier = prev.extra_info.get("tier_id").map(String::as_str);
    if prev_project != current_project_id || prev_tier != current_tier_id {
        return;
    }

    for prev_w in &prev.windows {
        if !is_bucket_window(prev_w) {
            continue;
        }
        let Some(canonical_key) = prev_w
            .window_key
            .as_deref()
            .and_then(canonical_bucket_key)
            .or_else(|| legacy_window_to_canonical_key(prev_w))
        else {
            continue;
        };
        if buckets
            .iter()
            .any(|cur| cur.window_key.as_deref() == Some(canonical_key))
        {
            continue;
        }
        let mut cloned = prev_w.clone();
        cloned.window_key = Some(canonical_key.to_string());
        cloned.window_kind = Some("bucket".to_string());
        cloned.stale = Some(true);
        buckets.push(cloned);
    }
}

fn apply_claude_weekly_blocking(
    buckets: &mut Vec<QuotaWindow>,
    is_free: bool,
    quota_summary_stale: bool,
    now: DateTime<Utc>,
) {
    if is_free || quota_summary_stale {
        return;
    }
    let Some(claude_weekly) = buckets
        .iter()
        .find(|w| w.window_key.as_deref() == Some("claude:weekly"))
    else {
        return;
    };
    if claude_weekly.summary_exhausted != Some(true) {
        return;
    }
    let reset_still_active = match claude_weekly.reset_time_iso.as_deref() {
        Some(iso) if !iso.trim().is_empty() => DateTime::parse_from_rfc3339(iso)
            .map(|dt| dt.with_timezone(&Utc) > now)
            .unwrap_or(true),
        _ => true,
    };
    if !reset_still_active {
        return;
    }

    if let Some(c5h) = buckets
        .iter_mut()
        .find(|w| w.window_key.as_deref() == Some("claude:5h"))
    {
        c5h.used_percent = None;
        c5h.remaining_percent = None;
        c5h.resets_at = None;
        c5h.reset_in_seconds = None;
        c5h.reset_time_iso = None;
        c5h.blocked_by_weekly = Some(true);
        c5h.detail = Some("周额度已用尽".to_string());
    } else {
        buckets.push(QuotaWindow {
            name: default_bucket_label("claude:5h"),
            used_percent: None,
            remaining_percent: None,
            resets_at: None,
            reset_in_seconds: None,
            detail: Some("周额度已用尽".to_string()),
            window_key: Some("claude:5h".to_string()),
            window_kind: Some("bucket".to_string()),
            stale: None,
            blocked_by_weekly: Some(true),
            summary_exhausted: None,
            reset_time_iso: None,
        });
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BuiltAntigravityQuota {
    pub windows: Vec<QuotaWindow>,
    pub quota_summary_stale: bool,
    pub is_forbidden: bool,
    pub error: Option<QuotaFetchError>,
}

fn build_antigravity_quota_from_responses(
    quota_response: &QuotaResponse,
    quota_summary: Option<&Value>,
    models_error: Option<QuotaFetchError>,
    quota_summary_error: Option<QuotaFetchError>,
    meta: &ProjectMetadataResult,
    previous_quota: Option<&AccountQuota>,
    now: DateTime<Utc>,
) -> BuiltAntigravityQuota {
    let summary_valid = quota_summary.is_some_and(is_summary_payload_valid);
    let quota_summary_stale = !summary_valid;

    let mut is_forbidden = false;
    if models_error.is_some() {
        is_forbidden = quota_summary_error.is_some();
    }

    let is_reauth = quota_summary_error.as_ref().is_some_and(is_reauth_error)
        || models_error.as_ref().is_some_and(is_reauth_error);

    let tier_code = normalize_subscription_tier_code(meta.tier_id.as_deref());
    let is_free = tier_code == Some("FREE");

    let mut bucket_windows = if summary_valid {
        parse_summary_buckets(quota_summary, is_free, now)
    } else {
        Vec::new()
    };

    let model_windows = parse_model_windows(quota_response, now);

    preserve_failed_quota_summary(
        &mut bucket_windows,
        quota_summary_stale,
        is_forbidden,
        is_reauth,
        meta.project_id.as_deref(),
        meta.tier_id.as_deref(),
        previous_quota,
    );

    if is_free {
        bucket_windows.retain(|w| {
            !w.window_key
                .as_deref()
                .is_some_and(|k| k.ends_with(":5h"))
        });
    }

    apply_claude_weekly_blocking(&mut bucket_windows, is_free, quota_summary_stale, now);

    bucket_windows.sort_by_key(|w| {
        bucket_order_rank(w.window_key.as_deref().unwrap_or(&w.name))
    });

    let effective_error = quota_summary_error.or_else(|| {
        models_error.filter(|_| bucket_windows.is_empty() && model_windows.is_empty())
    });

    let mut windows = bucket_windows;
    windows.extend(model_windows);

    BuiltAntigravityQuota {
        windows,
        quota_summary_stale,
        is_forbidden,
        error: effective_error,
    }
}

#[derive(Debug)]
enum FetchQuotaOutcome {
    Ok(BuiltAntigravityQuota),
    Unauthorized(String),
}

async fn post_cloudcode_json(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
    user_agent: &str,
    body: &Value,
) -> Result<reqwest::Response, String> {
    client
        .post(url)
        .bearer_auth(access_token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, user_agent)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Request to {} failed: {}", url, e))
}

async fn fetch_quota_from_base_url(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    email: Option<&str>,
    meta: &ProjectMetadataResult,
    previous_quota: Option<&AccountQuota>,
    now: DateTime<Utc>,
) -> Result<FetchQuotaOutcome, String> {
    let payload = valid_cloud_code_project_id(meta.project_id.as_deref())
        .map(|id| json!({ "project": id }))
        .unwrap_or_else(|| json!({}));
    let ua = build_cloud_code_user_agent();
    let models_url = format!("{}/{}", base_url, FETCH_AVAILABLE_MODELS_PATH);
    let summary_url = format!("{}/{}", base_url, RETRIEVE_USER_QUOTA_SUMMARY_PATH);

    let max_retries = 3;
    let mut models_error: Option<QuotaFetchError> = None;
    let mut quota_response = QuotaResponse {
        models: HashMap::new(),
    };
    let mut models_fetched = false;

    for attempt in 1..=max_retries {
        match post_cloudcode_json(client, &models_url, access_token, &ua, &payload).await {
            Ok(res) => {
                let status = res.status();
                if status == reqwest::StatusCode::FORBIDDEN {
                    let text = res.text().await.unwrap_or_default();
                    let err = parse_google_api_error_with_email(status.as_u16(), &text, email);
                    if is_explicit_quota_denial(err.reason.as_deref(), err.validation_url.as_deref())
                    {
                        let mut windows = Vec::new();
                        let reauth = is_reauth_error(&err);
                        if reauth {
                            preserve_failed_quota_summary(
                                &mut windows,
                                true,
                                true,
                                true,
                                meta.project_id.as_deref(),
                                meta.tier_id.as_deref(),
                                previous_quota,
                            );
                        }
                        return Ok(FetchQuotaOutcome::Ok(BuiltAntigravityQuota {
                            windows,
                            quota_summary_stale: true,
                            is_forbidden: true,
                            error: Some(err),
                        }));
                    }
                    models_error = Some(err);
                    models_fetched = true;
                    break;
                } else if status == reqwest::StatusCode::UNAUTHORIZED {
                    let text = res.text().await.unwrap_or_default();
                    return Ok(FetchQuotaOutcome::Unauthorized(format!(
                        "fetchAvailableModels failed: HTTP {} - {}",
                        status, text
                    )));
                } else if !status.is_success() {
                    let text = res.text().await.unwrap_or_default();
                    if attempt < max_retries {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        continue;
                    }
                    return Err(format!("fetchAvailableModels failed: HTTP {} - {}", status, text));
                } else {
                    quota_response = res
                        .json::<QuotaResponse>()
                        .await
                        .map_err(|e| format!("Failed to parse fetchAvailableModels: {}", e))?;
                    models_fetched = true;
                    break;
                }
            }
            Err(err) => {
                if attempt < max_retries {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
                return Err(err);
            }
        }
    }

    if !models_fetched {
        return Err("Failed to fetch Antigravity available models".to_string());
    }

    let mut quota_summary_val: Option<Value> = None;
    let mut quota_summary_error: Option<QuotaFetchError> = None;

    match post_cloudcode_json(client, &summary_url, access_token, &ua, &payload).await {
        Ok(res) => {
            let status = res.status();
            if status.is_success() {
                if let Ok(body) = res.text().await {
                    if let Ok(val) = serde_json::from_str::<Value>(&body) {
                        quota_summary_val = Some(val);
                    }
                }
            } else {
                let err_text = res.text().await.unwrap_or_default();
                let parsed = parse_google_api_error_with_email(status.as_u16(), &err_text, email);
                let explicitly_denied = if models_error.is_some() {
                    is_explicit_quota_denial(
                        parsed.reason.as_deref(),
                        parsed.validation_url.as_deref(),
                    )
                } else {
                    is_reauth_error(&parsed)
                };
                if explicitly_denied {
                    quota_summary_error = Some(parsed);
                } else {
                    eprintln!(
                        "[Antigravity] retrieveUserQuotaSummary returned HTTP {}: {}",
                        status, parsed.message
                    );
                }
            }
        }
        Err(err) => {
            eprintln!("[Antigravity] retrieveUserQuotaSummary request failed: {}", err);
        }
    }

    Ok(FetchQuotaOutcome::Ok(build_antigravity_quota_from_responses(
        &quota_response,
        quota_summary_val.as_ref(),
        models_error,
        quota_summary_error,
        meta,
        previous_quota,
        now,
    )))
}

pub async fn query_quota(
    payload_json: &mut String,
    account_id: &str,
    proxy: Option<&crate::core::models::ProxyConfig>,
) -> Result<AccountQuota, String> {
    query_quota_with_previous(payload_json, account_id, proxy, None).await
}

async fn resolve_metadata_and_fetch_quota(
    client: &reqwest::Client,
    val: &mut Value,
    payload_json: &mut String,
    access_token: &str,
    email: Option<&str>,
    previous_quota: Option<&AccountQuota>,
) -> Result<(ProjectMetadataResult, FetchQuotaOutcome), String> {
    let initial_ctx = QuotaCloudCodeContext::from_payload(val);
    let mut meta = fetch_project_metadata(client, access_token, &initial_ctx).await;

    if meta.project_id.is_none() {
        meta.project_id = initial_ctx.preferred_project_id.clone();
    }
    if meta.loaded_ok {
        if let Some(is_tos) = meta.is_gcp_tos {
            val["is_gcp_tos"] = Value::Bool(is_tos);
        }
    } else if meta.is_gcp_tos.is_none() && initial_ctx.is_gcp_tos {
        meta.is_gcp_tos = Some(true);
    }

    if let Some(ref pid) = meta.project_id {
        val["project_id"] = Value::String(pid.clone());
    }
    *payload_json = serde_json::to_string(val).unwrap_or_else(|_| payload_json.clone());

    let resolved_ctx = initial_ctx.after_project_discovery(&meta);
    let base_url = resolve_cloud_code_base_url(&resolved_ctx);

    let outcome = fetch_quota_from_base_url(
        client,
        &base_url,
        access_token,
        email,
        &meta,
        previous_quota,
        Utc::now(),
    )
    .await?;

    Ok((meta, outcome))
}

pub async fn query_quota_with_previous(
    payload_json: &mut String,
    account_id: &str,
    proxy: Option<&crate::core::models::ProxyConfig>,
    previous_quota: Option<&AccountQuota>,
) -> Result<AccountQuota, String> {
    let mut val: Value =
        serde_json::from_str(payload_json).map_err(|e| format!("Invalid JSON payload: {}", e))?;

    let mut access_token = ensure_fresh_access_token(&mut val, proxy).await?;
    *payload_json = serde_json::to_string(&val).unwrap_or_else(|_| payload_json.clone());

    let mut email = parse_email_from_payload(payload_json);
    let client = super::build_http_client(proxy, 15);

    if email.is_none() && !access_token.is_empty() {
        if let Ok(u_resp) = client
            .get("https://www.googleapis.com/oauth2/v3/userinfo")
            .bearer_auth(&access_token)
            .send()
            .await
        {
            if u_resp.status().is_success() {
                if let Ok(uinfo) = u_resp.json::<Value>().await {
                    if let Some(em) = uinfo.get("email").and_then(|e| e.as_str()) {
                        email = Some(em.to_string());
                        val["id_token_email"] = Value::String(em.to_string());
                    }
                }
            }
        }
    }

    let (mut meta, outcome) = resolve_metadata_and_fetch_quota(
        &client,
        &mut val,
        payload_json,
        &access_token,
        email.as_deref(),
        previous_quota,
    )
    .await?;

    let built = match outcome {
        FetchQuotaOutcome::Ok(b) => b,
        FetchQuotaOutcome::Unauthorized(err_msg) => {
            let refresh_token = val
                .pointer("/token/refresh_token")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if refresh_token.is_empty() {
                return Err(err_msg);
            }
            eprintln!("[Antigravity] Received 401, forcing access_token refresh and retrying");
            let (new_access, expires_in) = refresh_access_token(&refresh_token, proxy).await?;
            let new_expiry = (Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339();
            if let Some(token_obj) = val.get_mut("token") {
                token_obj["access_token"] = json!(new_access);
                token_obj["expiry"] = json!(new_expiry);
            }
            access_token = new_access;
            let (retry_meta, retry_outcome) = resolve_metadata_and_fetch_quota(
                &client,
                &mut val,
                payload_json,
                &access_token,
                email.as_deref(),
                previous_quota,
            )
            .await?;
            meta = retry_meta;
            match retry_outcome {
                FetchQuotaOutcome::Ok(b) => b,
                FetchQuotaOutcome::Unauthorized(retry_err) => return Err(retry_err),
            }
        }
    };

    let mut extra_info = HashMap::new();
    if let Some(ref tier_id) = meta.tier_id {
        extra_info.insert("tier_id".into(), tier_id.clone());
    }
    if let Some(tier_code) = normalize_subscription_tier_code(meta.tier_id.as_deref()) {
        extra_info.insert("subscription_tier".into(), tier_code.to_string());
    }
    if let Some(ref pid) = meta.project_id {
        extra_info.insert("project_id".into(), pid.clone());
    }
    if let Some(is_tos) = meta.is_gcp_tos {
        extra_info.insert("is_gcp_tos".into(), is_tos.to_string());
    }
    if let Some(ref credits) = meta.total_ai_credits {
        extra_info.insert("ai_credits".into(), credits.clone());
    }
    if built.quota_summary_stale {
        extra_info.insert("quota_summary_stale".into(), "true".into());
    }
    if built.is_forbidden {
        extra_info.insert("is_forbidden".into(), "true".into());
    }
    if let Some(ref err) = built.error {
        if let Some(code) = err.code {
            extra_info.insert("quota_error_code".into(), code.to_string());
        }
        extra_info.insert("quota_error_message".into(), err.message.clone());
        if let Some(ref reason) = err.reason {
            extra_info.insert("quota_error_reason".into(), reason.clone());
        }
        if let Some(ref url) = err.validation_url {
            extra_info.insert("validation_url".into(), url.clone());
        }
    }

    Ok(AccountQuota {
        provider: "antigravity".into(),
        account_id: account_id.to_string(),
        email,
        plan_type: meta.tier_display,
        fetched_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        windows: built.windows,
        extra_info,
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
    fn test_cloud_code_routing_matches_cockpit_tools() {
        let default_ctx = QuotaCloudCodeContext::default();
        assert_eq!(
            select_cloud_code_base_url(&default_ctx, None, false, false),
            CLOUD_CODE_DAILY_BASE_URL
        );

        let placeholder_gcp = QuotaCloudCodeContext {
            preferred_project_id: Some("aicode-consumers".into()),
            is_gcp_tos: true,
        };
        assert_eq!(
            select_cloud_code_base_url(&placeholder_gcp, None, false, false),
            CLOUD_CODE_DAILY_BASE_URL
        );

        let real_gcp = QuotaCloudCodeContext {
            preferred_project_id: Some("enterprise-proj-123".into()),
            is_gcp_tos: true,
        };
        assert_eq!(
            select_cloud_code_base_url(&real_gcp, None, false, false),
            CLOUD_CODE_PROD_BASE_URL
        );

        assert_eq!(
            select_cloud_code_base_url(&default_ctx, None, true, true),
            CLOUD_CODE_AUTOPUSH_SANDBOX_BASE_URL
        );
        assert_eq!(
            select_cloud_code_base_url(&real_gcp, Some("https://override.test"), true, true),
            "https://override.test"
        );
    }

    #[test]
    fn test_format_google_validation_url() {
        let raw = "https://accounts.google.com/signin/continue?sarp=1&authuser=0&continue=https%3A%2F%2Fdevelopers.google.com";
        let formatted = format_google_validation_url(raw, "target@gmail.com");
        assert!(formatted.contains("authuser=target%40gmail.com"));
        assert!(formatted.contains("login_hint=target%40gmail.com"));
        assert!(formatted.contains("Email=target%40gmail.com"));

        let evil = "https://evil-google.com/signin?authuser";
        assert_eq!(format_google_validation_url(evil, "target@gmail.com"), evil);
    }

    #[test]
    fn test_quota_summary_and_models_separation_and_rounding() {
        let raw_summary = json!({
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
                },
                {
                    "displayName": "Gemini 4 Carbon",
                    "buckets": [
                        {
                            "bucketId": "gemini-carbon-wtu-weekly",
                            "window": "weekly",
                            "resetTime": "2026-11-13T12:00:00Z",
                            "remainingFraction": 0.21
                        }
                    ]
                }
            ]
        });

        let raw_models: QuotaResponse = serde_json::from_value(json!({
            "models": {
                "gemini-4-hydrogen-high": {
                    "displayName": "Gemini 4 Hydrogen (High)",
                    "quotaInfo": { "remainingFraction": 0.39, "resetTime": "2026-11-06T12:35:34Z" }
                },
                "gemini-4-carbon-high": {
                    "displayName": "Gemini 4 Carbon (High)",
                    "quotaInfo": { "remainingFraction": 0.07, "resetTime": "2026-11-13T12:00:00Z" }
                },
                "chat_internal": {
                    "displayName": "Internal Chat",
                    "quotaInfo": { "remainingFraction": 1.0 }
                }
            }
        }))
        .unwrap();

        let meta = ProjectMetadataResult {
            loaded_ok: true,
            project_id: Some("proj-1".into()),
            tier_id: Some("g1-ultra-tier".into()),
            tier_display: Some("Ultra".into()),
            total_ai_credits: Some("25".into()),
            is_gcp_tos: Some(false),
        };

        let built = build_antigravity_quota_from_responses(
            &raw_models,
            Some(&raw_summary),
            None,
            None,
            &meta,
            None,
            fixed_now(),
        );

        assert!(!built.quota_summary_stale);
        assert!(!built.is_forbidden);

        let buckets: Vec<_> = built
            .windows
            .iter()
            .filter(|w| w.window_kind.as_deref() == Some("bucket"))
            .collect();
        assert_eq!(buckets.len(), 6);
        assert_eq!(buckets[0].window_key.as_deref(), Some("gemini:5h"));
        assert_eq!(buckets[0].remaining_percent, Some(100.0));
        assert_eq!(buckets[1].window_key.as_deref(), Some("gemini:weekly"));
        assert_eq!(buckets[1].remaining_percent, Some(67.0));
        assert_eq!(buckets[2].window_key.as_deref(), Some("claude:5h"));
        assert_eq!(buckets[2].remaining_percent, Some(0.0));
        assert_eq!(buckets[2].summary_exhausted, Some(true));
        assert_eq!(buckets[3].window_key.as_deref(), Some("claude:weekly"));
        assert_eq!(buckets[3].remaining_percent, Some(47.0));
        assert_eq!(buckets[4].window_key.as_deref(), Some("argon:weekly"));
        assert_eq!(buckets[4].remaining_percent, Some(87.0));
        assert_eq!(buckets[5].window_key.as_deref(), Some("carbon:weekly"));
        assert_eq!(buckets[5].remaining_percent, Some(21.0));

        let models: Vec<_> = built
            .windows
            .iter()
            .filter(|w| w.window_kind.as_deref() == Some("model"))
            .collect();
        assert_eq!(models.len(), 2);
        // Canonical order puts Carbon before Hydrogen
        assert_eq!(models[0].window_key.as_deref(), Some("gemini-4-carbon-high"));
        assert_eq!(models[0].remaining_percent, Some(7.0));
        assert_eq!(models[1].window_key.as_deref(), Some("gemini-4-hydrogen-high"));
        assert_eq!(models[1].remaining_percent, Some(39.0));
    }

    #[test]
    fn test_exhausted_claude_weekly_blocks_5h_using_raw_fraction() {
        let now = fixed_now();
        let meta = ProjectMetadataResult {
            tier_id: Some("g1-pro-tier".into()),
            tier_display: Some("Pro".into()),
            ..Default::default()
        };

        let exhausted_summary = json!({
            "groups": [{
                "buckets": [
                    { "bucketId": "3p-5h", "remainingFraction": 0.63, "resetTime": "2026-11-06T15:38:40Z" },
                    { "bucketId": "3p-weekly", "remainingFraction": 0.0, "resetTime": "2026-11-13T10:38:40Z" }
                ]
            }]
        });
        let built = build_antigravity_quota_from_responses(
            &QuotaResponse { models: HashMap::new() },
            Some(&exhausted_summary),
            None,
            None,
            &meta,
            None,
            now,
        );
        let c5h = built
            .windows
            .iter()
            .find(|w| w.window_key.as_deref() == Some("claude:5h"))
            .unwrap();
        assert_eq!(c5h.blocked_by_weekly, Some(true));
        assert_eq!(c5h.remaining_percent, None);

        // 0.004 rounds to 0% visually, but summary_exhausted must remain false so 5h stays active.
        let near_zero_summary = json!({
            "groups": [{
                "buckets": [
                    { "bucketId": "3p-5h", "remainingFraction": 0.63, "resetTime": "2026-11-06T15:38:40Z" },
                    { "bucketId": "3p-weekly", "remainingFraction": 0.004, "resetTime": "2026-11-13T10:38:40Z" }
                ]
            }]
        });
        let built2 = build_antigravity_quota_from_responses(
            &QuotaResponse { models: HashMap::new() },
            Some(&near_zero_summary),
            None,
            None,
            &meta,
            None,
            now,
        );
        let c5h_2 = built2
            .windows
            .iter()
            .find(|w| w.window_key.as_deref() == Some("claude:5h"))
            .unwrap();
        assert_ne!(c5h_2.blocked_by_weekly, Some(true));
        assert_eq!(c5h_2.remaining_percent, Some(63.0));
    }

    #[test]
    fn test_free_tier_omits_5h_and_preserves_stale_summary_without_model_pollution() {
        let now = fixed_now();
        let meta = ProjectMetadataResult {
            project_id: Some("p1".into()),
            tier_id: Some("free-tier".into()),
            tier_display: Some("Free".into()),
            ..Default::default()
        };

        let mut prev_extra = HashMap::new();
        prev_extra.insert("project_id".into(), "p1".into());
        prev_extra.insert("tier_id".into(), "free-tier".into());
        let prev = AccountQuota {
            provider: "antigravity".into(),
            account_id: "a1".into(),
            email: None,
            plan_type: Some("Free".into()),
            fetched_at: "2026-11-06 10:00:00".into(),
            windows: vec![QuotaWindow {
                name: "Claude · Weekly".into(),
                used_percent: Some(72.0),
                remaining_percent: Some(28.0),
                window_key: Some("claude:weekly".into()),
                window_kind: Some("bucket".into()),
                ..Default::default()
            }],
            extra_info: prev_extra,
        };

        let models: QuotaResponse = serde_json::from_value(json!({
            "models": {
                "gemini-3.1-pro-high": { "quotaInfo": { "remainingFraction": 1.0 } },
                "claude-sonnet-4-6": { "quotaInfo": { "remainingFraction": 1.0 } }
            }
        }))
        .unwrap();

        let built = build_antigravity_quota_from_responses(
            &models,
            None,
            None,
            None,
            &meta,
            Some(&prev),
            now,
        );
        assert!(built.quota_summary_stale);
        let buckets: Vec<_> = built
            .windows
            .iter()
            .filter(|w| w.window_kind.as_deref() == Some("bucket"))
            .collect();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].window_key.as_deref(), Some("claude:weekly"));
        assert_eq!(buckets[0].remaining_percent, Some(28.0));
        assert_eq!(buckets[0].stale, Some(true));
        // Fresh models stay separate and do NOT fabricate gemini:weekly or 5h buckets.
        let model_items: Vec<_> = built
            .windows
            .iter()
            .filter(|w| w.window_kind.as_deref() == Some("model"))
            .collect();
        assert_eq!(model_items.len(), 2);
    }

    #[test]
    fn test_cloud_code_user_agent_contains_aidev_client_and_cl() {
        let ua = build_cloud_code_user_agent();
        assert!(ua.contains("aidev_client"));
        assert!(ua.contains("cl=1003475615"));
    }

    #[test]
    fn test_upsert_bucket_entry_prefers_lower_raw_fraction_before_rounding() {
        let summary = json!({
            "groups": [{
                "buckets": [
                    { "bucketId": "3p-weekly", "remainingFraction": 0.004, "resetTime": "2026-11-13T10:38:40Z" },
                    { "bucketId": "claude:weekly", "remainingFraction": 0.0, "resetTime": "2026-11-13T10:38:40Z" }
                ]
            }]
        });
        let buckets = parse_summary_buckets(Some(&summary), false, fixed_now());
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].summary_exhausted, Some(true));
        assert_eq!(buckets[0].remaining_percent, Some(0.0));
    }

    async fn spawn_mock_cloudcode_server<F>(handler: F) -> (String, tokio::task::JoinHandle<()>)
    where
        F: Fn(&str, &str) -> (u16, Value) + Send + Sync + 'static,
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handler = std::sync::Arc::new(handler);

        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let Ok(n) = socket.read(&mut buf).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let first_line = req.lines().next().unwrap_or("");
                    let mut parts = first_line.split_whitespace();
                    let method = parts.next().unwrap_or("GET");
                    let path = parts.next().unwrap_or("/");
                    let (status, body) = handler(method, path);
                    let body_str = serde_json::to_string(&body).unwrap();
                    let status_text = match status {
                        200 => "OK",
                        401 => "Unauthorized",
                        403 => "Forbidden",
                        _ => "Error",
                    };
                    let resp = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        status,
                        status_text,
                        body_str.len(),
                        body_str
                    );
                    let _ = socket.write_all(resp.as_bytes()).await;
                });
            }
        });

        (format!("http://{}", addr), handle)
    }

    #[tokio::test]
    async fn test_http_load_code_assist_onboard_and_quota_pipeline() {
        let (base_url, server) = spawn_mock_cloudcode_server(|method, path| match (method, path) {
            ("POST", "/v1internal:loadCodeAssist") => (
                200,
                json!({
                    "cloudaicompanionProject": "aicode-consumers",
                    "paidTier": {
                        "id": "g1-pro-tier",
                        "availableCredits": [{ "creditType": "AI", "creditAmount": "25.50" }]
                    },
                    "currentTier": { "id": "free-tier", "usesGcpTos": false },
                    "allowedTiers": [{ "id": "g1-pro-tier", "isDefault": true, "usesGcpTos": false }]
                }),
            ),
            ("POST", "/v1internal:onboardUser") => (
                200,
                json!({ "name": "operations/op-1", "done": false }),
            ),
            ("GET", "/v1internal/operations/op-1") => (
                200,
                json!({ "done": true, "response": { "cloudaicompanionProject": { "id": "onboarded-proj" } } }),
            ),
            ("POST", "/v1internal:fetchAvailableModels") => (
                200,
                json!({
                    "models": {
                        "gemini-4-hydrogen-high": {
                            "displayName": "Gemini 4 Hydrogen (High)",
                            "quotaInfo": { "remainingFraction": 0.85, "resetTime": "2026-11-06T12:35:34Z" }
                        }
                    }
                }),
            ),
            ("POST", "/v1internal:retrieveUserQuotaSummary") => (
                200,
                json!({
                    "groups": [{
                        "buckets": [
                            { "bucketId": "gemini-5h", "remainingFraction": 0.9, "resetTime": "2026-11-06T12:35:34Z" },
                            { "bucketId": "gemini-weekly", "remainingFraction": 0.65, "resetTime": "2026-11-12T03:40:51Z" },
                            { "bucketId": "gemini-carbon-wtu-weekly", "remainingFraction": 0.5, "resetTime": "2026-11-12T03:40:51Z" }
                        ]
                    }]
                }),
            ),
            _ => (404, json!({})),
        })
        .await;

        let client = reqwest::Client::new();
        let meta = fetch_project_metadata_on_base_url(&client, &base_url, "tok", None)
            .await
            .expect("metadata loaded");
        assert!(meta.loaded_ok);
        assert_eq!(meta.project_id.as_deref(), Some("onboarded-proj"));
        assert_eq!(meta.tier_display.as_deref(), Some("Pro"));
        assert_eq!(meta.total_ai_credits.as_deref(), Some("25.5"));
        assert_eq!(meta.is_gcp_tos, Some(false));

        let outcome = fetch_quota_from_base_url(
            &client,
            &base_url,
            "tok",
            Some("dev@example.com"),
            &meta,
            None,
            fixed_now(),
        )
        .await
        .unwrap();
        server.abort();

        let FetchQuotaOutcome::Ok(built) = outcome else {
            panic!("expected Ok outcome");
        };
        assert!(!built.quota_summary_stale);
        assert!(built.windows.iter().any(|w| w.window_key.as_deref() == Some("carbon:weekly")));
        assert!(built.windows.iter().any(|w| w.window_key.as_deref() == Some("gemini-4-hydrogen-high")));
    }

    #[tokio::test]
    async fn test_http_ambiguous_403_and_validation_required_preserve_cache() {
        // Case A: ambiguous 403 Permission denied still calls retrieveUserQuotaSummary
        let (base_a, srv_a) = spawn_mock_cloudcode_server(|_, path| match path {
            "/v1internal:fetchAvailableModels" => (403, json!({ "error": { "message": "Permission denied" } })),
            "/v1internal:retrieveUserQuotaSummary" => (
                200,
                json!({ "groups": [{ "buckets": [{ "bucketId": "gemini-5h", "remainingFraction": 0.4, "resetTime": "2026-11-06T12:35:34Z" }] }] }),
            ),
            _ => (404, json!({})),
        })
        .await;

        let client = reqwest::Client::new();
        let meta = ProjectMetadataResult {
            loaded_ok: true,
            project_id: Some("p1".into()),
            tier_id: Some("g1-pro-tier".into()),
            tier_display: Some("Pro".into()),
            ..Default::default()
        };
        let FetchQuotaOutcome::Ok(built_a) = fetch_quota_from_base_url(
            &client,
            &base_a,
            "tok",
            Some("dev@example.com"),
            &meta,
            None,
            fixed_now(),
        )
        .await
        .unwrap()
        else {
            panic!("expected Ok outcome");
        };
        srv_a.abort();
        assert!(!built_a.is_forbidden);
        assert_eq!(built_a.windows.len(), 1);
        assert_eq!(built_a.windows[0].remaining_percent, Some(40.0));

        // Case B: VALIDATION_REQUIRED preserves cached buckets with stale=true and formats validation_url
        let (base_b, srv_b) = spawn_mock_cloudcode_server(|_, path| match path {
            "/v1internal:fetchAvailableModels" => (
                403,
                json!({
                    "error": {
                        "message": "Verify your account",
                        "details": [{
                            "reason": "VALIDATION_REQUIRED",
                            "metadata": { "validation_url": "https://accounts.google.com/signin/continue?authuser=0" }
                        }]
                    }
                }),
            ),
            _ => (404, json!({})),
        })
        .await;

        let mut prev_extra = HashMap::new();
        prev_extra.insert("project_id".into(), "p1".into());
        prev_extra.insert("tier_id".into(), "g1-pro-tier".into());
        let prev = AccountQuota {
            provider: "antigravity".into(),
            account_id: "a1".into(),
            email: Some("dev@example.com".into()),
            plan_type: Some("Pro".into()),
            fetched_at: "2026-11-06 10:00:00".into(),
            windows: vec![QuotaWindow {
                name: "Gemini · Weekly".into(),
                used_percent: Some(30.0),
                remaining_percent: Some(70.0),
                window_key: Some("gemini:weekly".into()),
                window_kind: Some("bucket".into()),
                ..Default::default()
            }],
            extra_info: prev_extra,
        };

        let FetchQuotaOutcome::Ok(built_b) = fetch_quota_from_base_url(
            &client,
            &base_b,
            "tok",
            Some("dev@example.com"),
            &meta,
            Some(&prev),
            fixed_now(),
        )
        .await
        .unwrap()
        else {
            panic!("expected Ok outcome");
        };
        srv_b.abort();
        assert!(built_b.is_forbidden);
        assert!(built_b.quota_summary_stale);
        assert_eq!(built_b.windows.len(), 1);
        assert_eq!(built_b.windows[0].stale, Some(true));
        assert_eq!(built_b.windows[0].remaining_percent, Some(70.0));
        let err = built_b.error.expect("error present");
        assert_eq!(err.reason.as_deref(), Some("VALIDATION_REQUIRED"));
        assert!(err
            .validation_url
            .as_deref()
            .unwrap()
            .contains("authuser=dev%40example.com"));
    }

    #[tokio::test]
    async fn test_http_401_returns_unauthorized_outcome_for_refresh() {
        let (base_url, server) = spawn_mock_cloudcode_server(|_, _| (401, json!({}))).await;
        let client = reqwest::Client::new();
        let outcome = fetch_quota_from_base_url(
            &client,
            &base_url,
            "expired-tok",
            None,
            &ProjectMetadataResult::default(),
            None,
            fixed_now(),
        )
        .await
        .unwrap();
        server.abort();
        assert!(matches!(outcome, FetchQuotaOutcome::Unauthorized(_)));
    }
}
