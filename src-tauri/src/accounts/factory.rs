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

pub const AUTH_FILES: &[&str] = &["auth.v2.file", "auth.v2.key", "auth.encrypted", "auth.v2.keyring"];

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
    let payload: AuthPayload = match serde_json::from_str(payload_json) {
        Ok(p) => p,
        Err(_) => return id,
    };

    if let Some(claims_val) = decode_jwt_claims(&payload.access_token) {
        if let Ok(claims) = serde_json::from_value::<JwtClaims>(claims_val.clone()) {
            id.email = claims.email;
            id.subject = claims.sub;
            id.expires_at = claims.exp.and_then(|s| DateTime::from_timestamp(s, 0));

            let raw_plan = claims.plan
                .or(claims.plan_type)
                .or(claims.tier)
                .or(claims.subscription);
            if let Some(p) = raw_plan {
                let formatted = match p.to_lowercase().as_str() {
                    "pro" => "Pro".to_string(),
                    "plus" => "Plus".to_string(),
                    "max" => "Max".to_string(),
                    "team" | "teams" => "Team".to_string(),
                    "enterprise" => "Enterprise".to_string(),
                    _ => p,
                };
                id.plan_type = Some(formatted);
            }
        }
    }
    id
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
        return Err(format!("未在 {} 中检测到 Factory 登录凭据，请先在 Factory Desktop 或 Droid 客户端中完成登录。", factory_dir.display()));
    }

    // Attempt decryption
    let mut decrypted_json: Option<String> = None;

    let v2_bytes_opt = files.get("auth.v2.file").cloned();
    if let Some(v2_bytes) = v2_bytes_opt {
        let v2_str = String::from_utf8_lossy(&v2_bytes).to_string();

        // 1. Check if auth.v2.key is on disk
        #[allow(unused_mut)]
        let mut key_raw: Option<Vec<u8>> = files.get("auth.v2.key").and_then(|k| {
            let key_str = String::from_utf8_lossy(k).trim().to_string();
            B64.decode(key_str).ok()
        });

        // 2. Fallback to OS keyring (Windows Credential Manager) if auth.v2.key is missing
        #[cfg(target_os = "windows")]
        if key_raw.is_none() {
            for target_name in &["Factory CLI/auth-encryption-key", "Factory CLI"] {
                if let Ok(blob) = crate::accounts::crypto::cred_mgr::read_credential(target_name) {
                    if blob.len() == 32 {
                        // Persist to auth.v2.key for seamless portability
                        let b64 = B64.encode(&blob);
                        let _ = fs::create_dir_all(&factory_dir);
                        let _ = fs::write(factory_dir.join("auth.v2.key"), &b64);
                        files.insert("auth.v2.key".to_string(), b64.into_bytes());
                        key_raw = Some(blob);
                        break;
                    } else if let Ok(s) = String::from_utf8(blob) {
                        let trimmed = s.trim();
                        if let Ok(decoded) = B64.decode(trimmed) {
                            if decoded.len() == 32 {
                                let _ = fs::create_dir_all(&factory_dir);
                                let _ = fs::write(factory_dir.join("auth.v2.key"), trimmed);
                                files.insert("auth.v2.key".to_string(), trimmed.as_bytes().to_vec());
                                key_raw = Some(decoded);
                                break;
                            }
                        }
                    }
                }
            }
        }

        if let Some(key) = key_raw {
            if let Ok(decrypted) = decrypt_v2_payload(&v2_str, &key) {
                decrypted_json = Some(decrypted);
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
        // Attempt decryption from saved files
        let v2_bytes = files.get("auth.v2.file").ok_or_else(|| "auth.v2.file missing in saved account".to_string())?;
        let key_bytes = files.get("auth.v2.key").ok_or_else(|| "auth.v2.key missing in saved account".to_string())?;
        let v2_str = String::from_utf8_lossy(v2_bytes);
        let key_str = String::from_utf8_lossy(key_bytes).trim().to_string();
        let key_raw = B64.decode(&key_str).map_err(|e| e.to_string())?;
        decrypt_v2_payload(&v2_str, &key_raw)?
    };

    Ok(FactoryAuthBundle {
        decrypted_json,
        files,
    })
}

pub fn restore_bundle_to_live(provider: &str, account_id: &str) -> Result<(), String> {
    let bundle = load_bundle_from_storage(provider, account_id)?;
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
            }).to_string()
        } else {
            val.to_string()
        }
    } else {
        // Assume raw JWT access token
        serde_json::json!({
            "access_token": trimmed,
            "refresh_token": "",
        }).to_string()
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

pub async fn query_quota(
    bundle: &FactoryAuthBundle,
    account_id: &str,
    proxy: Option<&ProxyConfig>,
) -> Result<AccountQuota, String> {
    let identity = parse_identity(&bundle.decrypted_json);
    let now = Utc::now();
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let mut windows = Vec::new();
    let mut extra_info = HashMap::new();

    extra_info.insert("Provider".into(), "Factory Droid".into());
    if let Some(ref em) = identity.email {
        extra_info.insert("Email".into(), em.clone());
    }
    if let Some(ref sub) = identity.subject {
        extra_info.insert("Subject ID".into(), sub.clone());
    }

    // 1. Session Token Expiry Window
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

    // 2. Factory Rolling Rate Limit Windows
    // Factory documentation defines:
    // - 5-Hour Rolling Limit (Standard Usage consumed first)
    // - 7-Day Rolling Limit (Fallback to Droid Core open-weight models on exhaustion)
    // - 30-Day Rolling Limit (Monthly subscription reset & Extra Usage buffer)
    let plan = identity.plan_type.clone().unwrap_or_else(|| "Pro / Standard".to_string());
    extra_info.insert("Plan Tier".into(), plan.clone());

    // Probe live Factory API endpoint if access token is available
    let mut api_credit_info = None;
    let mut user_profile_info = None;

    if let Ok(auth_payload) = serde_json::from_str::<AuthPayload>(&bundle.decrypted_json) {
        let client = super::build_http_client(proxy, 6);

        // 1. Try Organization limits endpoint (for Team / Enterprise tiers)
        let org_url = "https://api.factory.ai/api/v0/organization/usage/limits/users";
        if let Ok(resp) = client.get(org_url)
            .bearer_auth(&auth_payload.access_token)
            .header("User-Agent", "Local-AI-Route/Factory-Client")
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(val) = resp.json::<Value>().await {
                    api_credit_info = Some(val);
                }
            }
        }

        // 2. Try User info endpoint (for Individual Pro / Standard tiers)
        let user_url = "https://api.factory.ai/api/v0/users/me";
        if let Ok(resp) = client.get(user_url)
            .bearer_auth(&auth_payload.access_token)
            .header("User-Agent", "Local-AI-Route/Factory-Client")
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(val) = resp.json::<Value>().await {
                    user_profile_info = Some(val);
                }
            }
        }
    }

    if let Some(ref u) = user_profile_info {
        if let Some(email) = u.get("email").and_then(|v| v.as_str()) {
            extra_info.insert("User Email".into(), email.to_string());
        }
        if let Some(sub) = u.get("subscription").or_else(|| u.get("tier")).and_then(|v| v.as_str()) {
            extra_info.insert("Subscription Tier".into(), sub.to_string());
        }
        if let Some(status) = u.get("status").and_then(|v| v.as_str()) {
            extra_info.insert("Account Status".into(), status.to_string());
        }
    }

    if let Some(ref info) = api_credit_info {
        if let Some(global_lim) = info.get("globalLimit").and_then(|v| v.as_i64()) {
            extra_info.insert("Global Credits Limit".into(), global_lim.to_string());
        }
        if let Some(used) = info.get("usedCredits").and_then(|v| v.as_i64()) {
            extra_info.insert("Used Credits".into(), used.to_string());
        }
    }

    // 5-Hour Rolling Rate Limit Window
    windows.push(QuotaWindow {
        name: "5小时滚动速率限制".into(),
        used_percent: None,
        remaining_percent: Some(100.0),
        resets_at: None,
        reset_in_seconds: None,
        detail: Some(format!("{} · 5小时滚动速率窗口，Standard Usage 优先消耗", plan)),
    });

    // 7-Day Rolling Rate Limit Window
    windows.push(QuotaWindow {
        name: "7天滚动速率限制".into(),
        used_percent: None,
        remaining_percent: Some(100.0),
        resets_at: None,
        reset_in_seconds: None,
        detail: Some("7天速率窗口 · 超额自动降级至免费 Droid Core 开源模型池".into()),
    });

    // 30-Day Rolling Cycle
    windows.push(QuotaWindow {
        name: "30天计费重置周期".into(),
        used_percent: None,
        remaining_percent: Some(100.0),
        resets_at: None,
        reset_in_seconds: None,
        detail: Some("月度滚动重置周期 · 支持额外充值 Extra Usage 永不过期".into()),
    });

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
        for target in &["Factory CLI/auth-encryption-key", "Factory CLI"] {
            let _ = crate::accounts::crypto::cred_mgr::delete_credential(target);
        }
    }

    Ok("已重置 Factory 桌面端/Droid 当前会话凭据与系统凭据，您可以重新启动 Factory 客户端登录新账号，登录完成后点击「从当前客户端导入」即可保存。".into())
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
        assert_eq!(id_team.plan_type.as_deref(), Some("Team"));

        let payload_ent = r#"{"access_token":"eyJhbGciOiJub25lIn0.eyJzdWJzY3JpcHRpb24iOiJlbnRlcnByaXNlIn0."}"#;
        let id_ent = parse_identity(payload_ent);
        assert_eq!(id_ent.plan_type.as_deref(), Some("Enterprise"));
    }

    #[test]
    fn test_decode_jwt_padding_variants() {
        // Unpadded URL-safe token
        let token = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let claims = decode_jwt_claims(token).expect("claims decoded");
        assert_eq!(claims.get("name").and_then(|v| v.as_str()), Some("John Doe"));
    }
}
