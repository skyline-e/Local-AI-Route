use super::models::{AccountQuota, QuotaWindow};
use super::storage;
use crate::core::models::ProxyConfig;
use aes_gcm::aead::generic_array::typenum::{U12, U16};
use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::aead::Aead;
use aes_gcm::aes::Aes256;
use aes_gcm::{AesGcm, KeyInit};
use base64::engine::general_purpose::STANDARD as B64;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

type Aes256Gcm12 = AesGcm<Aes256, U12>;
type Aes256Gcm16 = AesGcm<Aes256, U16>;

pub const AUTH_FILES: &[&str] = &[
    "auth.v2.file",
    "auth.v2.key",
    "auth.encrypted",
    "auth.v2.keyring",
    "auth.v2.loginkeychain",
];

const WORKOS_CLIENT_ID_PROD: &str = "client_01HNM792M5G5G1A2THWPXKFMXB";
const WORKOS_AUTH_URL: &str = "https://api.workos.com/user_management/authenticate";
const FACTORY_API_BASES: &[&str] = &["https://api.factory.ai", "https://app.factory.ai"];

#[cfg(target_os = "windows")]
const CRED_MGR_TARGETS: &[&str] = &[
    "Factory CLI/auth-encryption-key",
    "Factory CLI",
    "Factory Desktop/auth-encryption-key",
    "Factory/auth-encryption-key",
];

pub fn get_factory_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("FACTORY_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Ok(dir) = std::env::var("DSW_HOME") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Some(home) = dirs::home_dir() {
        let f_dir = home.join(".factory");
        if f_dir.is_dir() {
            return f_dir;
        }
        let dsw_dir = home.join(".dsw");
        if dsw_dir.is_dir() {
            return dsw_dir;
        }
        return f_dir;
    }
    PathBuf::from(".factory")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryAuthBundle {
    pub decrypted_json: String,
    pub files: HashMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct FactoryIdentity {
    pub email: Option<String>,
    pub subject: Option<String>,
    pub plan_type: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct AuthPayload {
    access_token: String,
    #[allow(dead_code)]
    refresh_token: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct JwtClaims {
    email: Option<String>,
    sub: Option<String>,
    exp: Option<i64>,
    iat: Option<i64>,
    plan: Option<String>,
    plan_type: Option<String>,
    tier: Option<String>,
    subscription: Option<String>,
    name: Option<String>,
}

/// Maps internal Factory tier identifiers (`factoryTier`) to official UI display names.
pub fn format_factory_tier(raw_tier: &str) -> String {
    match raw_tier.trim().to_lowercase().as_str() {
        "team" | "pro" => "Pro".to_string(),
        "team_annual" | "pro_annual" => "Pro Annual".to_string(),
        "pro_plus" | "plus" => "Plus".to_string(),
        "max" => "Max".to_string(),
        "ultra" => "Ultra".to_string(),
        "teams" => "Teams".to_string(),
        "payg" => "Enterprise (PAYG)".to_string(),
        "business" => "Business".to_string(),
        "enterprise" => "Enterprise".to_string(),
        "free" | "starter" => "Free".to_string(),
        _ => raw_tier.trim().to_string(),
    }
}

pub fn decrypt_v2_payload(raw_file: &str, raw_key: &[u8]) -> Result<String, String> {
    if raw_key.len() != 32 {
        return Err(format!("auth.v2.key must be 32 bytes (got {})", raw_key.len()));
    }

    let parts: Vec<&str> = raw_file.trim().split(':').collect();
    if parts.len() != 3 {
        return Err(format!("auth.v2.file must have 3 colon-separated segments, got {}", parts.len()));
    }
    let iv = B64.decode(parts[0]).map_err(|e| format!("decode iv failed: {}", e))?;
    let tag = B64.decode(parts[1]).map_err(|e| format!("decode tag failed: {}", e))?;
    let ct = B64.decode(parts[2]).map_err(|e| format!("decode ciphertext failed: {}", e))?;

    let mut combined = ct;
    combined.extend_from_slice(&tag);

    let pt = match iv.len() {
        12 => {
            let cipher = Aes256Gcm12::new_from_slice(raw_key).map_err(|e| format!("invalid key: {}", e))?;
            let nonce = GenericArray::<u8, U12>::from_slice(&iv);
            cipher.decrypt(nonce, combined.as_ref())
        }
        16 => {
            let cipher = Aes256Gcm16::new_from_slice(raw_key).map_err(|e| format!("invalid key: {}", e))?;
            let nonce = GenericArray::<u8, U16>::from_slice(&iv);
            cipher.decrypt(nonce, combined.as_ref())
        }
        n => return Err(format!("unsupported IV length {} (expected 12 or 16)", n)),
    }
    .map_err(|e| format!("AES-GCM decryption failed: {}", e))?;

    String::from_utf8(pt).map_err(|e| format!("decrypted payload is not UTF-8: {}", e))
}

pub fn encrypt_v2_payload(json_str: &str, existing_key: Option<&[u8]>) -> Result<(String, String), String> {
    use aes_gcm::aead::rand_core::RngCore;
    use aes_gcm::aead::OsRng;

    let mut key_bytes = [0u8; 32];
    if let Some(k) = existing_key {
        if k.len() == 32 {
            key_bytes.copy_from_slice(k);
        } else {
            OsRng.fill_bytes(&mut key_bytes);
        }
    } else {
        OsRng.fill_bytes(&mut key_bytes);
    }

    let mut iv = [0u8; 16];
    OsRng.fill_bytes(&mut iv);

    let cipher = Aes256Gcm16::new_from_slice(&key_bytes).map_err(|e| format!("cipher init failed: {}", e))?;
    let nonce = GenericArray::<u8, U16>::from_slice(&iv);
    let ct_with_tag = cipher.encrypt(nonce, json_str.as_bytes()).map_err(|e| format!("encrypt failed: {}", e))?;
    if ct_with_tag.len() < 16 {
        return Err("Ciphertext too short".into());
    }
    let (ct, tag) = ct_with_tag.split_at(ct_with_tag.len() - 16);

    let file_contents = format!("{}:{}:{}", B64.encode(iv), B64.encode(tag), B64.encode(ct));
    let key_b64 = B64.encode(key_bytes);
    Ok((file_contents, key_b64))
}

pub fn decode_jwt_claims(jwt: &str) -> Option<Value> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload = parts[1];
    let decoded = B64URL
        .decode(payload)
        .or_else(|_| {
            let mut s = payload.replace('-', "+").replace('_', "/");
            let pad = (4 - (s.len() % 4)) % 4;
            s.push_str(&"=".repeat(pad));
            B64.decode(s)
        })
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

pub fn parse_identity(payload_json: &str) -> FactoryIdentity {
    let mut id = FactoryIdentity::default();
    let payload_val: Value = match serde_json::from_str(payload_json) {
        Ok(v) => v,
        Err(_) => return id,
    };

    let access_token = payload_val
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if !access_token.is_empty() {
        if let Some(claims_val) = decode_jwt_claims(access_token) {
            if let Ok(claims) = serde_json::from_value::<JwtClaims>(claims_val) {
                id.email = claims.email;
                id.subject = claims.sub;
                id.expires_at = claims.exp.and_then(|s| DateTime::from_timestamp(s, 0));

                let raw_plan = claims
                    .plan
                    .or(claims.plan_type)
                    .or(claims.tier)
                    .or(claims.subscription);
                if let Some(p) = raw_plan {
                    id.plan_type = Some(format_factory_tier(&p));
                }
            }
        }
    }

    // Fallback to cached whoami object in Factory Desktop auth payload if present
    if let Some(whoami) = payload_val.get("whoami") {
        if id.email.is_none() {
            id.email = whoami
                .get("email")
                .or_else(|| whoami.pointer("/user/email"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
        if id.subject.is_none() {
            id.subject = whoami
                .get("userId")
                .or_else(|| whoami.pointer("/user/id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
        if id.plan_type.is_none() {
            if let Some(tier) = whoami
                .pointer("/organization/subscription/factoryTier")
                .and_then(|v| v.as_str())
            {
                id.plan_type = Some(format_factory_tier(tier));
            }
        }
    }

    id
}

fn resolve_v2_encryption_key(
    files: &mut HashMap<String, Vec<u8>>,
    factory_dir: Option<&PathBuf>,
) -> Option<Vec<u8>> {
    // 1. Check if auth.v2.key or auth.v2.loginkeychain is in files
    for key_fname in &["auth.v2.key", "auth.v2.loginkeychain"] {
        if let Some(k) = files.get(*key_fname) {
            let key_str = String::from_utf8_lossy(k).trim().to_string();
            if let Ok(decoded) = B64.decode(&key_str) {
                if decoded.len() == 32 {
                    return Some(decoded);
                }
            }
            if k.len() == 32 {
                return Some(k.clone());
            }
        }
    }

    // 2. Fallback to OS keyring (Windows Credential Manager) if key file is missing
    #[cfg(target_os = "windows")]
    {
        for target_name in CRED_MGR_TARGETS {
            if let Ok(blob) = crate::accounts::crypto::cred_mgr::read_credential(target_name) {
                if blob.len() == 32 {
                    let b64 = B64.encode(&blob);
                    if let Some(dir) = factory_dir {
                        let _ = fs::create_dir_all(dir);
                        let _ = fs::write(dir.join("auth.v2.key"), &b64);
                    }
                    files.insert("auth.v2.key".to_string(), b64.into_bytes());
                    return Some(blob);
                } else if let Ok(s) = String::from_utf8(blob) {
                    let trimmed = s.trim();
                    if let Ok(decoded) = B64.decode(trimmed) {
                        if decoded.len() == 32 {
                            if let Some(dir) = factory_dir {
                                let _ = fs::create_dir_all(dir);
                                let _ = fs::write(dir.join("auth.v2.key"), trimmed);
                            }
                            files.insert("auth.v2.key".to_string(), trimmed.as_bytes().to_vec());
                            return Some(decoded);
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    let _ = factory_dir;

    None
}

pub fn read_live_auth_bundle() -> Result<FactoryAuthBundle, String> {
    let factory_dir = get_factory_dir();
    if !factory_dir.is_dir() {
        return Err(format!("Factory 目录未找到: {}", factory_dir.display()));
    }

    let mut files = HashMap::new();
    for fname in AUTH_FILES {
        let p = factory_dir.join(fname);
        if p.is_file() {
            if let Ok(bytes) = fs::read(&p) {
                files.insert(fname.to_string(), bytes);
            }
        }
    }

    if files.is_empty() {
        return Err(format!(
            "未在 {} 中检测到 Factory 登录凭据，请先在 Factory Desktop 客户端中完成登录。",
            factory_dir.display()
        ));
    }

    let mut decrypted_json: Option<String> = None;
    let key_raw = resolve_v2_encryption_key(&mut files, Some(&factory_dir));

    if let Some(ref key) = key_raw {
        for enc_fname in &["auth.v2.file", "auth.v2.keyring"] {
            if let Some(enc_bytes) = files.get(*enc_fname) {
                let enc_str = String::from_utf8_lossy(enc_bytes);
                if let Ok(decrypted) = decrypt_v2_payload(&enc_str, key) {
                    decrypted_json = Some(decrypted);
                    break;
                }
            }
        }
    }

    // Fallback: check auth.encrypted (legacy plaintext JSON)
    if decrypted_json.is_none() {
        if let Some(enc_bytes) = files.get("auth.encrypted") {
            if let Ok(s) = String::from_utf8(enc_bytes.clone()) {
                if serde_json::from_str::<AuthPayload>(&s).is_ok() {
                    decrypted_json = Some(s);
                }
            }
        }
    }

    let payload = decrypted_json.ok_or_else(|| {
        "解密 Factory 凭证失败。可能是密钥文件 auth.v2.key 损坏或系统凭据管理未提供有效密钥。".to_string()
    })?;

    Ok(FactoryAuthBundle {
        decrypted_json: payload,
        files,
    })
}

pub fn save_bundle_to_storage(provider: &str, account_id: &str, bundle: &FactoryAuthBundle) -> Result<(), String> {
    for (fname, bytes) in &bundle.files {
        storage::save_account_data(provider, account_id, fname, bytes)?;
    }
    // Also save decrypted JSON for fast offline loading & inspection
    storage::save_account_data(provider, account_id, "payload.json", bundle.decrypted_json.as_bytes())?;
    Ok(())
}

pub fn load_bundle_from_storage(provider: &str, account_id: &str) -> Result<FactoryAuthBundle, String> {
    let mut files = HashMap::new();
    for fname in AUTH_FILES {
        if let Ok(bytes) = storage::read_account_data(provider, account_id, fname) {
            files.insert(fname.to_string(), bytes);
        }
    }

    let decrypted_json = if let Ok(pbytes) = storage::read_account_data(provider, account_id, "payload.json") {
        String::from_utf8(pbytes).map_err(|e| e.to_string())?
    } else {
        let key_raw = resolve_v2_encryption_key(&mut files, None)
            .ok_or_else(|| "auth.v2.key missing in saved account".to_string())?;
        let v2_bytes = files
            .get("auth.v2.file")
            .or_else(|| files.get("auth.v2.keyring"))
            .ok_or_else(|| "auth.v2.file missing in saved account".to_string())?;
        let v2_str = String::from_utf8_lossy(v2_bytes);
        decrypt_v2_payload(&v2_str, &key_raw)?
    };

    Ok(FactoryAuthBundle {
        decrypted_json,
        files,
    })
}

pub fn restore_bundle_to_live(provider: &str, account_id: &str) -> Result<(), String> {
    let bundle = load_bundle_from_storage(provider, account_id)?;
    write_bundle_to_live(&bundle)
}

pub fn write_bundle_to_live(bundle: &FactoryAuthBundle) -> Result<(), String> {
    let factory_dir = get_factory_dir();
    fs::create_dir_all(&factory_dir).map_err(|e| format!("Failed to create Factory directory: {}", e))?;

    // Copy profile files over live ~/.factory
    for &fname in AUTH_FILES {
        let dest = factory_dir.join(fname);
        if let Some(bytes) = bundle.files.get(fname) {
            fs::write(&dest, bytes).map_err(|e| format!("Failed to restore {}: {}", fname, e))?;
        } else if dest.is_file() {
            // Remove file not belonging to target account to prevent stale credential contamination
            let _ = fs::remove_file(dest);
        }
    }

    // On Windows, keep OS Credential Manager in sync if key is present
    #[cfg(target_os = "windows")]
    if let Some(key_bytes) = bundle.files.get("auth.v2.key") {
        let key_str = String::from_utf8_lossy(key_bytes).trim().to_string();
        if let Ok(raw_32) = B64.decode(&key_str) {
            if raw_32.len() == 32 {
                let _ = crate::accounts::crypto::cred_mgr::write_credential(
                    "Factory CLI/auth-encryption-key",
                    "auth-encryption-key",
                    &raw_32,
                );
            }
        }
    }

    Ok(())
}

pub fn create_bundle_from_raw(raw_content: &str) -> Result<FactoryAuthBundle, String> {
    let trimmed = raw_content.trim();
    let auth_json_str = if trimmed.starts_with('{') {
        let val: Value = serde_json::from_str(trimmed)
            .map_err(|e| format!("Invalid JSON format for Factory auth: {}", e))?;
        if val.get("access_token").is_none() && val.get("token").is_some() {
            serde_json::json!({
                "access_token": val.get("token").and_then(|t| t.as_str()).unwrap_or(""),
                "refresh_token": val.get("refresh_token").and_then(|r| r.as_str()).unwrap_or(""),
            })
            .to_string()
        } else {
            val.to_string()
        }
    } else {
        // Assume raw JWT access token
        serde_json::json!({
            "access_token": trimmed,
            "refresh_token": "",
        })
        .to_string()
    };

    let (v2_file, v2_key) = encrypt_v2_payload(&auth_json_str, None)?;
    let mut files = HashMap::new();
    files.insert("auth.v2.file".to_string(), v2_file.into_bytes());
    files.insert("auth.v2.key".to_string(), v2_key.into_bytes());
    files.insert("auth.encrypted".to_string(), auth_json_str.as_bytes().to_vec());

    Ok(FactoryAuthBundle {
        decrypted_json: auth_json_str,
        files,
    })
}

fn update_bundle_with_new_tokens(
    bundle: &mut FactoryAuthBundle,
    new_access_token: &str,
    new_refresh_token: Option<&str>,
) -> Result<(), String> {
    let mut root_val: Value = serde_json::from_str(&bundle.decrypted_json)
        .unwrap_or_else(|_| serde_json::json!({}));

    if let Some(obj) = root_val.as_object_mut() {
        obj.insert("access_token".to_string(), Value::String(new_access_token.to_string()));
        if let Some(rt) = new_refresh_token {
            if !rt.is_empty() {
                obj.insert("refresh_token".to_string(), Value::String(rt.to_string()));
            }
        }
    }

    let updated_json = serde_json::to_string(&root_val).map_err(|e| e.to_string())?;
    let existing_key = resolve_v2_encryption_key(&mut bundle.files, None);

    let (v2_file, v2_key) = encrypt_v2_payload(&updated_json, existing_key.as_deref())?;
    bundle.decrypted_json = updated_json.clone();
    bundle.files.insert("auth.v2.file".to_string(), v2_file.clone().into_bytes());
    bundle.files.insert("auth.v2.key".to_string(), v2_key.clone().into_bytes());
    if bundle.files.contains_key("auth.v2.loginkeychain") {
        bundle.files.insert("auth.v2.loginkeychain".to_string(), v2_key.into_bytes());
    }
    if bundle.files.contains_key("auth.v2.keyring") {
        bundle.files.insert("auth.v2.keyring".to_string(), v2_file.into_bytes());
    }
    if bundle.files.contains_key("auth.encrypted") {
        bundle
            .files
            .insert("auth.encrypted".to_string(), updated_json.into_bytes());
    }

    Ok(())
}

/// Refreshes the Factory WorkOS access token using `refresh_token` when expired or forced.
/// Returns `Ok(true)` if the token was refreshed and `bundle` was updated.
pub async fn refresh_bundle_token(
    bundle: &mut FactoryAuthBundle,
    proxy: Option<&ProxyConfig>,
    force: bool,
) -> Result<bool, String> {
    let root_val: Value = serde_json::from_str(&bundle.decrypted_json)
        .map_err(|e| format!("Invalid Factory auth JSON: {}", e))?;

    let access_token = root_val
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let refresh_token = root_val
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let now_ts = Utc::now().timestamp();
    let mut needs_refresh = force || access_token.is_empty();
    let mut client_id = WORKOS_CLIENT_ID_PROD.to_string();

    if !access_token.is_empty() {
        if let Some(claims) = decode_jwt_claims(access_token) {
            if let Some(exp) = claims.get("exp").and_then(|v| v.as_i64()) {
                // Refresh if token expires within 60 seconds
                if exp <= now_ts + 60 {
                    needs_refresh = true;
                }
            }
            if let Some(cid) = claims
                .get("client_id")
                .and_then(|v| v.as_str())
                .filter(|s| s.starts_with("client_"))
            {
                client_id = cid.to_string();
            }
        }
    }

    if !needs_refresh {
        return Ok(false);
    }

    if refresh_token.is_empty() {
        return Err("Factory access_token 已过期且未包含 refresh_token，无法自动刷新".into());
    }

    let org_id = root_val
        .get("active_organization_id")
        .and_then(|v| v.as_str())
        .or_else(|| root_val.pointer("/whoami/orgId").and_then(|v| v.as_str()))
        .unwrap_or("");

    let mut form_params: Vec<(&str, &str)> = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", &client_id),
    ];
    if !org_id.is_empty() {
        form_params.push(("organization_id", org_id));
    }

    let client = super::build_http_client(proxy, 10);
    let resp = client
        .post(WORKOS_AUTH_URL)
        .header("User-Agent", "Factory Desktop")
        .form(&form_params)
        .send()
        .await
        .map_err(|e| format!("WorkOS token refresh request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("WorkOS token refresh failed ({}): {}", status, body));
    }

    let data: Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid JSON from WorkOS refresh: {}", e))?;

    let new_at = data
        .get("access_token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "WorkOS refresh response missing access_token".to_string())?;
    let new_rt = data.get("refresh_token").and_then(|v| v.as_str());

    update_bundle_with_new_tokens(bundle, new_at, new_rt)?;
    Ok(true)
}

/// Parses a single Factory rate limit bucket (`fiveHour`, `weekly`, `monthly`),
/// applying the same expiration semantics as Factory Desktop (`Sgt(bucket)` in `app.asar`).
pub fn parse_rate_limit_bucket(
    bucket: Option<&Value>,
    now: DateTime<Utc>,
) -> (f64, f64, Option<String>, Option<i64>) {
    let Some(b) = bucket else {
        return (0.0, 100.0, None, None);
    };

    let window_end_str = b.get("windowEnd").and_then(|v| v.as_str());
    let window_end_dt = window_end_str.and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|dt| dt.with_timezone(&Utc));

    let sec_rem_opt = b
        .get("secondsRemaining")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)))
        .or_else(|| window_end_dt.map(|dt| dt.timestamp() - now.timestamp()));

    // A bucket is expired/inactive if windowEnd is null or secondsRemaining <= 0
    let is_active = match (window_end_str, sec_rem_opt) {
        (Some(_), Some(sec)) if sec > 0 => true,
        _ => false,
    };

    if !is_active {
        return (0.0, 100.0, None, None);
    }

    let raw_used = b
        .get("usedPercent")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        .clamp(0.0, 100.0);
    let used_pct = (raw_used * 10.0).round() / 10.0;
    let rem_pct = ((100.0 - used_pct).clamp(0.0, 100.0) * 10.0).round() / 10.0;

    let resets_at = window_end_dt
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string());
    let reset_in_seconds = sec_rem_opt.map(|s| s.max(0));

    (used_pct, rem_pct, resets_at, reset_in_seconds)
}

async fn fetch_factory_endpoint(
    client: &reqwest::Client,
    access_token: &str,
    path: &str,
) -> Result<(reqwest::StatusCode, Option<Value>), String> {
    let mut last_status = reqwest::StatusCode::INTERNAL_SERVER_ERROR;
    let mut last_err = String::new();

    for base in FACTORY_API_BASES {
        let url = format!("{}{}", base, path);
        match client
            .get(&url)
            .bearer_auth(access_token)
            .header("User-Agent", "Factory Desktop")
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                last_status = status;
                if status.is_success() {
                    if let Ok(val) = resp.json::<Value>().await {
                        return Ok((status, Some(val)));
                    }
                } else if status == reqwest::StatusCode::UNAUTHORIZED {
                    return Ok((status, None));
                }
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
    }

    if !last_err.is_empty() && last_status == reqwest::StatusCode::INTERNAL_SERVER_ERROR {
        return Err(last_err);
    }
    Ok((last_status, None))
}

pub async fn query_quota(
    bundle: &mut FactoryAuthBundle,
    account_id: &str,
    proxy: Option<&ProxyConfig>,
) -> Result<AccountQuota, String> {
    // 1. Proactively refresh token if expired or expiring within 60s
    let _ = refresh_bundle_token(bundle, proxy, false).await;

    let mut identity = parse_identity(&bundle.decrypted_json);
    let now = Utc::now();
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let mut windows = Vec::new();
    let mut extra_info = HashMap::new();

    extra_info.insert("Provider".into(), "Factory Desktop".into());

    let mut access_token = serde_json::from_str::<AuthPayload>(&bundle.decrypted_json)
        .map(|p| p.access_token)
        .unwrap_or_default();

    let client = super::build_http_client(proxy, 8);
    let mut limits_val: Option<Value> = None;
    let mut org_val: Option<Value> = None;

    if !access_token.is_empty() {
        let (limits_res, org_res) = tokio::join!(
            fetch_factory_endpoint(&client, &access_token, "/api/billing/limits"),
            fetch_factory_endpoint(&client, &access_token, "/api/organization"),
        );

        let mut hit_unauthorized = false;
        if let Ok((status, val_opt)) = limits_res {
            if status == reqwest::StatusCode::UNAUTHORIZED {
                hit_unauthorized = true;
            } else {
                limits_val = val_opt;
            }
        }
        if let Ok((status, val_opt)) = org_res {
            if status == reqwest::StatusCode::UNAUTHORIZED {
                hit_unauthorized = true;
            } else {
                org_val = val_opt;
            }
        }

        // Retry once concurrently with forced token refresh if either endpoint returned 401
        if hit_unauthorized && refresh_bundle_token(bundle, proxy, true).await.unwrap_or(false) {
            identity = parse_identity(&bundle.decrypted_json);
            access_token = serde_json::from_str::<AuthPayload>(&bundle.decrypted_json)
                .map(|p| p.access_token)
                .unwrap_or_default();
            let (retry_limits, retry_org) = tokio::join!(
                fetch_factory_endpoint(&client, &access_token, "/api/billing/limits"),
                fetch_factory_endpoint(&client, &access_token, "/api/organization"),
            );
            if let Ok((_, val_opt)) = retry_limits {
                limits_val = val_opt;
            }
            if let Ok((_, val_opt)) = retry_org {
                org_val = val_opt;
            }
        }
    }

    // Fallback to cached whoami.organization in bundle if offline
    if org_val.is_none() {
        if let Ok(root_val) = serde_json::from_str::<Value>(&bundle.decrypted_json) {
            if let Some(cached_org) = root_val.pointer("/whoami/organization") {
                org_val = Some(serde_json::json!({ "organization": cached_org }));
            }
        }
    }

    // 2. Parse Organization & Subscription details
    let mut plan = identity.plan_type.clone().unwrap_or_else(|| "Pro".to_string());
    if let Some(ref o_root) = org_val {
        let org_obj = o_root.get("organization").unwrap_or(o_root);
        if let Some(org_name) = org_obj.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            extra_info.insert("Organization".into(), org_name.to_string());
        }
        if let Some(tier) = org_obj
            .pointer("/subscription/factoryTier")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            plan = format_factory_tier(tier);
        }
        if let Some(orb) = org_obj.pointer("/subscription/orbSubscription") {
            if let Some(sub_name) = orb.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                extra_info.insert("Subscription Plan".into(), sub_name.to_string());
            }
            if let Some(status) = orb.get("status").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                extra_info.insert("Subscription Status".into(), status.to_string());
            }
            let period_end = orb
                .get("current_billing_period_end_date")
                .and_then(|v| v.as_str())
                .or_else(|| orb.get("end_date").and_then(|v| v.as_str()));
            if let Some(end_str) = period_end {
                if let Ok(end_dt) = DateTime::parse_from_rfc3339(end_str) {
                    let end_utc = end_dt.with_timezone(&Utc);
                    let rem_sec = (end_utc.timestamp() - now.timestamp()).max(0);
                    let is_expired = end_utc <= now;
                    let local_str = end_utc.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string();
                    extra_info.insert("subscription_expires_at".into(), local_str);
                    extra_info.insert("subscription_sec_remaining".into(), rem_sec.to_string());
                    extra_info.insert(
                        "subscription_is_expired".into(),
                        if is_expired { "true" } else { "false" }.into(),
                    );
                }
            }
        }
    }

    if let Some(ref em) = identity.email {
        extra_info.insert("Email".into(), em.clone());
    }
    if let Some(ref sub) = identity.subject {
        extra_info.insert("Subject ID".into(), sub.clone());
    }

    // 3. Session Token Expiry Metadata
    if let Some(exp) = identity.expires_at {
        let exp_ts = exp.timestamp();
        let rem_sec = (exp_ts - now.timestamp()).max(0);
        let is_expired = exp <= now;
        let resets_at_str = exp.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string();

        let detail_str = if is_expired {
            "已过期".to_string()
        } else {
            let dur = exp - now;
            let days = dur.num_days();
            if days >= 1 {
                format!("有效 (剩余 {} 天)", days)
            } else {
                format!("有效 (剩余 {} 小时)", dur.num_hours().max(1))
            }
        };

        extra_info.insert("token_expires_at".into(), resets_at_str);
        extra_info.insert("token_sec_remaining".into(), rem_sec.to_string());
        extra_info.insert("token_is_expired".into(), if is_expired { "true" } else { "false" }.into());
        extra_info.insert("token_status".into(), detail_str);
    }

    extra_info.insert("Plan Tier".into(), plan.clone());

    // 4. Parse Usage Limits Windows (/api/billing/limits)
    if let Some(ref lim_root) = limits_val {
        let overage_pref = lim_root
            .get("overagePreference")
            .and_then(|v| v.as_str())
            .unwrap_or("droidCore");
        extra_info.insert("Overage Preference".into(), overage_pref.to_string());

        if let Some(extra_cents) = lim_root.get("extraUsageBalanceCents").and_then(|v| v.as_i64()) {
            extra_info.insert(
                "Extra Usage Balance".into(),
                format!("${:.2}", (extra_cents as f64) / 100.0),
            );
        }

        let standard = lim_root.pointer("/limits/standard");
        let (fh_used, fh_rem, fh_resets_at, fh_reset_sec) =
            parse_rate_limit_bucket(standard.and_then(|s| s.get("fiveHour")), now);
        let (wk_used, wk_rem, wk_resets_at, wk_reset_sec) =
            parse_rate_limit_bucket(standard.and_then(|s| s.get("weekly")), now);
        let (mo_used, mo_rem, mo_resets_at, mo_reset_sec) =
            parse_rate_limit_bucket(standard.and_then(|s| s.get("monthly")), now);

        windows.push(QuotaWindow {
            name: "5-hour usage (5小时用量)".into(),
            used_percent: Some(fh_used),
            remaining_percent: Some(fh_rem),
            resets_at: fh_resets_at,
            reset_in_seconds: fh_reset_sec,
            detail: Some(format!("{} · Standard 5小时滚动用量窗口", plan)),
        });

        windows.push(QuotaWindow {
            name: "Weekly usage (周用量)".into(),
            used_percent: Some(wk_used),
            remaining_percent: Some(wk_rem),
            resets_at: wk_resets_at,
            reset_in_seconds: wk_reset_sec,
            detail: Some(format!(
                "{} · Standard 7天滚动用量窗口 (超额策略: {})",
                plan, overage_pref
            )),
        });

        windows.push(QuotaWindow {
            name: "Monthly usage (月度用量)".into(),
            used_percent: Some(mo_used),
            remaining_percent: Some(mo_rem),
            resets_at: mo_resets_at,
            reset_in_seconds: mo_reset_sec,
            detail: Some(format!("{} · Standard 月度计费用量窗口", plan)),
        });

        // Also surface Droid Core rate limits if any Droid Core bucket is actively used
        if let Some(core) = lim_root.pointer("/limits/core") {
            for (key, label) in &[
                ("fiveHour", "Droid Core · 5-hour usage"),
                ("weekly", "Droid Core · Weekly usage"),
                ("monthly", "Droid Core · Monthly usage"),
            ] {
                let (c_used, c_rem, c_resets_at, c_reset_sec) =
                    parse_rate_limit_bucket(core.get(*key), now);
                if c_used > 0.0 || c_reset_sec.is_some() {
                    windows.push(QuotaWindow {
                        name: (*label).into(),
                        used_percent: Some(c_used),
                        remaining_percent: Some(c_rem),
                        resets_at: c_resets_at,
                        reset_in_seconds: c_reset_sec,
                        detail: Some("Droid Core 免费开源模型池速率窗口".into()),
                    });
                }
            }
        }
    } else {
        // Offline / API unreachable fallback
        windows.push(QuotaWindow {
            name: "5-hour usage (5小时用量)".into(),
            used_percent: None,
            remaining_percent: None,
            resets_at: None,
            reset_in_seconds: None,
            detail: Some("暂无法连接 Factory Billing API，请检查网络或代理设置后点击刷新配额".into()),
        });
    }

    Ok(AccountQuota {
        provider: "factory".into(),
        account_id: account_id.to_string(),
        email: identity.email,
        plan_type: Some(plan),
        fetched_at: now_str,
        windows,
        extra_info,
    })
}

pub fn prepare_new_factory_login() -> Result<String, String> {
    let factory_dir = get_factory_dir();
    if factory_dir.is_dir() {
        for fname in AUTH_FILES {
            let p = factory_dir.join(fname);
            if p.is_file() {
                let _ = fs::remove_file(p);
            }
        }
        for extra in &["session.json", "auth.lock", ".cache"] {
            let p = factory_dir.join(extra);
            if p.is_file() {
                let _ = fs::remove_file(p);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        for target in CRED_MGR_TARGETS {
            let _ = crate::accounts::crypto::cred_mgr::delete_credential(target);
        }
    }

    Ok("已重置 Factory 桌面端当前会话凭据与系统凭据，您可以重新启动 Factory 客户端登录新账号，登录完成后点击「从当前客户端导入」即可保存。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_v2_16_byte_iv() {
        let json_data = r#"{"access_token":"test.jwt.token","refresh_token":"rt_123"}"#;
        let (v2_file, v2_key) = encrypt_v2_payload(json_data, None).expect("encrypt ok");
        let key_raw = B64.decode(v2_key.trim()).expect("decode key");
        let decrypted = decrypt_v2_payload(&v2_file, &key_raw).expect("decrypt ok");
        assert_eq!(decrypted, json_data);
    }

    #[test]
    fn test_decrypt_v2_12_byte_iv() {
        use aes_gcm::aead::rand_core::RngCore;
        use aes_gcm::aead::OsRng;
        let json_data = r#"{"access_token":"test.jwt.12iv","refresh_token":"rt_456"}"#;
        let mut key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut key_bytes);
        let mut iv_12 = [0u8; 12];
        OsRng.fill_bytes(&mut iv_12);

        let cipher = Aes256Gcm12::new_from_slice(&key_bytes).expect("cipher init");
        let nonce = GenericArray::<u8, U12>::from_slice(&iv_12);
        let ct_with_tag = cipher.encrypt(nonce, json_data.as_bytes()).expect("encrypt");
        let (ct, tag) = ct_with_tag.split_at(ct_with_tag.len() - 16);
        let file_contents = format!("{}:{}:{}", B64.encode(iv_12), B64.encode(tag), B64.encode(ct));

        let decrypted = decrypt_v2_payload(&file_contents, &key_bytes).expect("decrypt 12-byte IV");
        assert_eq!(decrypted, json_data);
    }

    #[test]
    fn test_create_bundle_from_raw() {
        let jwt = "eyJhbGciOiJub25lIn0.eyJlbWFpbCI6ImFsaWNlQGV4YW1wbGUuY29tIiwic3ViIjoidXNlcl8xIiwiZXhwIjoxODAwMDAwMDAwfQ.";
        let bundle = create_bundle_from_raw(jwt).expect("create bundle ok");
        let id = parse_identity(&bundle.decrypted_json);
        assert_eq!(id.email.as_deref(), Some("alice@example.com"));
        assert_eq!(id.subject.as_deref(), Some("user_1"));
    }

    #[test]
    fn test_parse_identity_plan_variations() {
        let payload_pro = r#"{"access_token":"eyJhbGciOiJub25lIn0.eyJlbWFpbCI6ImJvYkBleGFtcGxlLmNvbSIsInN1YiI6InVzZXJfMiIsInBsYW4iOiJwcm8iLCJpYXQiOjE2MDAwMDAwMDAsImV4cCI6MTgwMDAwMDAwMH0."}"#;
        let id_pro = parse_identity(payload_pro);
        assert_eq!(id_pro.email.as_deref(), Some("bob@example.com"));
        assert_eq!(id_pro.plan_type.as_deref(), Some("Pro"));

        let payload_team = r#"{"access_token":"eyJhbGciOiJub25lIn0.eyJlbWFpbCI6ImFkbWluQHRlYW0uY29tIiwidGllciI6InRlYW1zIiwiZXhwIjoxODAwMDAwMDAwfQ."}"#;
        let id_team = parse_identity(payload_team);
        assert_eq!(id_team.plan_type.as_deref(), Some("Teams"));

        let payload_ent = r#"{"access_token":"eyJhbGciOiJub25lIn0.eyJzdWJzY3JpcHRpb24iOiJlbnRlcnByaXNlIn0."}"#;
        let id_ent = parse_identity(payload_ent);
        assert_eq!(id_ent.plan_type.as_deref(), Some("Enterprise"));

        // Verify fallback to whoami.organization.subscription.factoryTier ("team" -> "Pro")
        let payload_whoami = r#"{"access_token":"","whoami":{"email":"dev@factory.ai","userId":"usr_99","organization":{"subscription":{"factoryTier":"team"}}}}"#;
        let id_whoami = parse_identity(payload_whoami);
        assert_eq!(id_whoami.email.as_deref(), Some("dev@factory.ai"));
        assert_eq!(id_whoami.subject.as_deref(), Some("usr_99"));
        assert_eq!(id_whoami.plan_type.as_deref(), Some("Pro"));
    }

    #[test]
    fn test_parse_rate_limit_bucket() {
        let now = DateTime::parse_from_rfc3339("2026-11-09T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let active_bucket = serde_json::json!({
            "usedPercent": 16,
            "windowEnd": "2026-11-09T13:43:10.091Z",
            "secondsRemaining": 17028
        });
        let (used, rem, resets_at, sec) = parse_rate_limit_bucket(Some(&active_bucket), now);
        assert_eq!(used, 16.0);
        assert_eq!(rem, 84.0);
        assert!(resets_at.is_some());
        assert_eq!(sec, Some(17028));

        let inactive_bucket = serde_json::json!({
            "usedPercent": 0,
            "windowEnd": null,
            "secondsRemaining": null
        });
        let (used2, rem2, resets_at2, sec2) = parse_rate_limit_bucket(Some(&inactive_bucket), now);
        assert_eq!(used2, 0.0);
        assert_eq!(rem2, 100.0);
        assert!(resets_at2.is_none());
        assert!(sec2.is_none());
    }

    #[test]
    fn test_decode_jwt_padding_variants() {
        // Unpadded URL-safe token
        let token = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let claims = decode_jwt_claims(token).expect("claims decoded");
        assert_eq!(claims.get("name").and_then(|v| v.as_str()), Some("John Doe"));
    }
}
