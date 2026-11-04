pub mod antigravity;
pub mod chatgpt;
pub mod claude;
pub mod crypto;
pub mod factory;
pub mod models;
pub mod storage;

use crate::core::controller::{ensure_stopped_before_relaunch, launch_target};
use crate::core::models::{ProxyConfig, TargetAppSpec};
use models::{AccountProfile, AccountQuota, AccountSwitchResult};
use uuid::Uuid;

pub fn normalize_provider(app_id: &str) -> Option<&'static str> {
    match app_id {
        "antigravity-app" | "antigravity-ide" | "antigravity" => Some("antigravity"),
        "chatgpt" => Some("chatgpt"),
        "claude" => Some("claude"),
        "factory" | "factory-app" | "factory-desktop" | "droid" => Some("factory"),
        _ => None,
    }
}

pub fn build_http_client(proxy: Option<&ProxyConfig>, timeout_secs: u64) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs));
    if let Some(p) = proxy {
        let proxy_url = p.url();
        if let Ok(reqwest_proxy) = reqwest::Proxy::all(&proxy_url) {
            builder = builder.proxy(reqwest_proxy);
        }
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

pub fn list_accounts(app_id: &str) -> Result<Vec<AccountProfile>, String> {
    let provider = normalize_provider(app_id)
        .ok_or_else(|| format!("Provider for app '{}' does not support multi-account", app_id))?;
    storage::load_accounts(provider)
}

pub async fn import_current_account(
    app_id: &str,
    custom_name: Option<String>,
    proxy: Option<&ProxyConfig>,
) -> Result<AccountProfile, String> {
    let provider = normalize_provider(app_id)
        .ok_or_else(|| format!("App '{}' is not supported for account management", app_id))?;

    let mut existing = storage::load_accounts(provider).unwrap_or_default();
    let account_id = Uuid::new_v4().to_string();
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    match provider {
        "antigravity" => {
            let mut payload = antigravity::read_current_credential_payload()?;
            let mut email = antigravity::parse_email_from_payload(&payload);
            let mut display_name = custom_name.clone().unwrap_or_else(|| {
                if let Some(ref em) = email {
                    em.split('@').next().unwrap_or("Antigravity Account").to_string()
                } else {
                    "Antigravity Account".into()
                }
            });

            // Check if already exists by email OR if existing single account has no email yet (in-place upgrade)
            let existing_pos = if let Some(ref em) = email {
                existing.iter().position(|a| a.email.as_deref() == Some(em))
                    .or_else(|| if existing.len() == 1 && existing[0].email.is_none() { Some(0) } else { None })
            } else if existing.len() == 1 && existing[0].email.is_none() {
                Some(0)
            } else {
                None
            };

            if let Some(pos) = existing_pos {
                // Update existing payload
                let existing_id = existing[pos].id.clone();
                storage::save_account_data(provider, &existing_id, "credential.json", payload.as_bytes())?;
                for a in &mut existing {
                    a.is_active = a.id == existing_id;
                }
                existing[pos].last_used_at = Some(now_str.clone());
                if email.is_some() {
                    existing[pos].email = email.clone();
                    existing[pos].account_id = email.clone();
                    if custom_name.is_none() && existing[pos].name == "Antigravity Account" {
                        if let Some(ref em) = email {
                            existing[pos].name = em.split('@').next().unwrap_or("Antigravity Account").to_string();
                        }
                    }
                }
                let mut quota = None;
                if let Ok(q) = antigravity::query_quota(&mut payload, &existing_id, proxy).await {
                    let _ = storage::save_account_data(provider, &existing_id, "credential.json", payload.as_bytes());
                    if existing[pos].email.is_none() && q.email.is_some() {
                        existing[pos].email = q.email.clone();
                        existing[pos].account_id = q.email.clone();
                        if custom_name.is_none() && existing[pos].name == "Antigravity Account" {
                            if let Some(ref em) = q.email {
                                existing[pos].name = em.split('@').next().unwrap_or("Antigravity Account").to_string();
                            }
                        }
                    }
                    if q.plan_type.is_some() {
                        existing[pos].plan_type = q.plan_type.clone();
                    }
                    quota = Some(q);
                }
                existing[pos].quota = quota;
                let _ = storage::save_accounts(provider, &existing);
                return Ok(existing[pos].clone());
            }

            // Save credential payload file
            storage::save_account_data(provider, &account_id, "credential.json", payload.as_bytes())?;

            let mut quota = None;
            if let Ok(q) = antigravity::query_quota(&mut payload, &account_id, proxy).await {
                // write back possibly refreshed payload
                let _ = storage::save_account_data(provider, &account_id, "credential.json", payload.as_bytes());
                if email.is_none() && q.email.is_some() {
                    email = q.email.clone();
                    if custom_name.is_none() {
                        if let Some(ref em) = email {
                            display_name = em.split('@').next().unwrap_or("Antigravity Account").to_string();
                        }
                    }
                }
                quota = Some(q);
            }

            // Set all others inactive
            for a in &mut existing {
                a.is_active = false;
            }

            let profile = AccountProfile {
                id: account_id,
                provider: provider.into(),
                name: display_name,
                email: email.clone(),
                account_id: email,
                plan_type: quota.as_ref().and_then(|q| q.plan_type.clone()),
                created_at: now_str.clone(),
                last_used_at: Some(now_str),
                is_active: true,
                quota,
            };

            existing.push(profile.clone());
            storage::save_accounts(provider, &existing)?;
            Ok(profile)
        }
        "chatgpt" => {
            let auth_json = chatgpt::read_current_auth_json()?;
            let identity = chatgpt::parse_identity_from_auth(&auth_json);
            let display_name = custom_name.unwrap_or_else(|| {
                if let Some(ref em) = identity.email {
                    em.split('@').next().unwrap_or("ChatGPT Account").to_string()
                } else {
                    "ChatGPT Account".into()
                }
            });

            if let Some(ref em) = identity.email {
                if let Some(pos) = existing.iter().position(|a| a.email.as_deref() == Some(em)) {
                    let existing_id = existing[pos].id.clone();
                    storage::save_account_data(provider, &existing_id, "auth.json", auth_json.as_bytes())?;
                    for a in &mut existing {
                        a.is_active = a.id == existing_id;
                    }
                    existing[pos].last_used_at = Some(now_str.clone());
                    let _ = storage::save_accounts(provider, &existing);
                    return Ok(existing[pos].clone());
                }
            }

            storage::save_account_data(provider, &account_id, "auth.json", auth_json.as_bytes())?;

            let quota = chatgpt::query_quota(&auth_json, &account_id, proxy).await.ok();

            for a in &mut existing {
                a.is_active = false;
            }

            let profile = AccountProfile {
                id: account_id.clone(),
                provider: provider.into(),
                name: display_name,
                email: identity.email.clone(),
                account_id: identity.account_id.clone().or(identity.email.clone()),
                plan_type: identity.plan_type.clone().or_else(|| quota.as_ref().and_then(|q| q.plan_type.clone())),
                created_at: now_str.clone(),
                last_used_at: Some(now_str),
                is_active: true,
                quota,
            };

            existing.push(profile.clone());
            storage::save_accounts(provider, &existing)?;
            Ok(profile)
        }
        "claude" => {
            let claude_dir = claude::find_claude_data_dir()
                .ok_or_else(|| "Could not find Claude desktop data directory on this machine".to_string())?;

            let cfg_path = claude_dir.join("config.json");
            let cfg_raw = std::fs::read_to_string(&cfg_path)
                .map_err(|e| format!("Failed to read Claude config.json: {}", e))?;

            let cfg_val: serde_json::Value = serde_json::from_str(&cfg_raw)
                .map_err(|e| format!("Failed to parse Claude config.json: {}", e))?;

            let mut auth_keys = serde_json::Map::new();
            for k in &["oauth:tokenCache", "oauth:tokenCacheV2", "lastKnownAccountUuid"] {
                if let Some(v) = cfg_val.get(*k) {
                    auth_keys.insert(k.to_string(), v.clone());
                }
            }

            if auth_keys.get("oauth:tokenCacheV2").is_none() {
                return Err("Claude is not signed in (no oauth:tokenCacheV2 found)".into());
            }

            let auth_json_str = serde_json::to_string_pretty(&auth_keys).map_err(|e| e.to_string())?;

            // Save auth keys
            storage::save_account_data(provider, &account_id, "config_auth.json", auth_json_str.as_bytes())?;

            // Save cookies
            for f in claude::get_claude_cookie_files(&claude_dir) {
                if f.is_file() {
                    if let Ok(bytes) = std::fs::read(&f) {
                        if let Some(fname) = f.file_name().and_then(|n| n.to_str()) {
                            let _ = storage::save_account_data(provider, &account_id, fname, &bytes);
                        }
                    }
                }
            }

            let quota = claude::query_quota(&claude_dir, &auth_json_str, &account_id, proxy).await.ok();
            let email = quota.as_ref().and_then(|q| q.email.clone());
            let display_name = custom_name.unwrap_or_else(|| {
                if let Some(ref em) = email {
                    em.split('@').next().unwrap_or("Claude Account").to_string()
                } else {
                    "Claude Account".into()
                }
            });

            if let Some(ref em) = email {
                if let Some(pos) = existing.iter().position(|a| a.email.as_deref() == Some(em)) {
                    let existing_id = existing[pos].id.clone();
                    // Synchronize auth config and fresh cookies to existing account
                    storage::save_account_data(provider, &existing_id, "config_auth.json", auth_json_str.as_bytes())?;
                    for f in claude::get_claude_cookie_files(&claude_dir) {
                        if f.is_file() {
                            if let Ok(bytes) = std::fs::read(&f) {
                                if let Some(fname) = f.file_name().and_then(|n| n.to_str()) {
                                    let _ = storage::save_account_data(provider, &existing_id, fname, &bytes);
                                }
                            }
                        }
                    }
                    // Clean up temporary account_id files
                    let _ = storage::delete_account_files(provider, &account_id);

                    for a in &mut existing {
                        a.is_active = a.id == existing_id;
                    }
                    existing[pos].last_used_at = Some(now_str.clone());
                    if quota.is_some() {
                        existing[pos].quota = quota.clone();
                        if let Some(ref q) = quota {
                            if q.plan_type.is_some() {
                                existing[pos].plan_type = q.plan_type.clone();
                            }
                        }
                    }
                    let _ = storage::save_accounts(provider, &existing);
                    return Ok(existing[pos].clone());
                }
            }

            for a in &mut existing {
                a.is_active = false;
            }

            let profile = AccountProfile {
                id: account_id.clone(),
                provider: provider.into(),
                name: display_name,
                email: email.clone(),
                account_id: auth_keys.get("lastKnownAccountUuid").and_then(|v| v.as_str()).map(|s| s.to_string()),
                plan_type: quota.as_ref().and_then(|q| q.plan_type.clone()),
                created_at: now_str.clone(),
                last_used_at: Some(now_str),
                is_active: true,
                quota,
            };

            existing.push(profile.clone());
            storage::save_accounts(provider, &existing)?;
            Ok(profile)
        }
        "factory" => {
            let bundle = factory::read_live_auth_bundle()?;
            let identity = factory::parse_identity(&bundle.decrypted_json);
            let display_name = custom_name.unwrap_or_else(|| {
                if let Some(ref em) = identity.email {
                    em.split('@').next().unwrap_or("Factory Account").to_string()
                } else {
                    "Factory Account".into()
                }
            });

            if let Some(ref em) = identity.email {
                if let Some(pos) = existing.iter().position(|a| a.email.as_deref() == Some(em)) {
                    let existing_id = existing[pos].id.clone();
                    factory::save_bundle_to_storage(provider, &existing_id, &bundle)?;
                    for a in &mut existing {
                        a.is_active = a.id == existing_id;
                    }
                    existing[pos].last_used_at = Some(now_str.clone());
                    let quota = factory::query_quota(&bundle, &existing_id, proxy).await.ok();
                    if let Some(ref q) = quota {
                        existing[pos].quota = Some(q.clone());
                        if q.plan_type.is_some() {
                            existing[pos].plan_type = q.plan_type.clone();
                        }
                    }
                    let _ = storage::save_accounts(provider, &existing);
                    return Ok(existing[pos].clone());
                }
            }

            factory::save_bundle_to_storage(provider, &account_id, &bundle)?;
            let quota = factory::query_quota(&bundle, &account_id, proxy).await.ok();

            for a in &mut existing {
                a.is_active = false;
            }

            let profile = AccountProfile {
                id: account_id.clone(),
                provider: provider.into(),
                name: display_name,
                email: identity.email.clone(),
                account_id: identity.subject.clone().or_else(|| identity.email.clone()),
                plan_type: identity.plan_type.clone().or_else(|| quota.as_ref().and_then(|q| q.plan_type.clone())),
                created_at: now_str.clone(),
                last_used_at: Some(now_str),
                is_active: true,
                quota,
            };

            existing.push(profile.clone());
            storage::save_accounts(provider, &existing)?;
            Ok(profile)
        }
        _ => Err("Unsupported provider".into()),
    }
}

pub async fn add_account_manual(
    app_id: &str,
    name: String,
    raw_content: String,
    proxy: Option<&ProxyConfig>,
) -> Result<AccountProfile, String> {
    let provider = normalize_provider(app_id)
        .ok_or_else(|| format!("App '{}' is not supported", app_id))?;

    let mut existing = storage::load_accounts(provider).unwrap_or_default();
    let is_first = existing.is_empty();
    let account_id = Uuid::new_v4().to_string();
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let raw_trimmed = raw_content.trim();

    match provider {
        "antigravity" => {
            // Support both full JSON and raw refresh_token string
            let val_result: Result<serde_json::Value, _> = serde_json::from_str(raw_trimmed);
            let mut val = match val_result {
                Ok(v) => v,
                Err(_) if !raw_trimmed.starts_with('{') => {
                    serde_json::json!({
                        "token": {
                            "refresh_token": raw_trimmed,
                            "access_token": ""
                        },
                        "auth_method": "consumer"
                    })
                }
                Err(e) => return Err(format!("Invalid JSON format for Antigravity credential: {}", e)),
            };

            // Normalize structure if user only pasted token { access_token, refresh_token }
            if val.get("token").is_none() && val.get("refresh_token").is_some() {
                val = serde_json::json!({
                    "token": val,
                    "auth_method": "consumer"
                });
            }

            let mut serialized = serde_json::to_string(&val).map_err(|e| e.to_string())?;
            let email = antigravity::parse_email_from_payload(&serialized);

            storage::save_account_data(provider, &account_id, "credential.json", serialized.as_bytes())?;

            let quota = antigravity::query_quota(&mut serialized, &account_id, proxy).await.ok();
            let _ = storage::save_account_data(provider, &account_id, "credential.json", serialized.as_bytes());

            let profile = AccountProfile {
                id: account_id,
                provider: provider.into(),
                name,
                email: email.clone(),
                account_id: email,
                plan_type: quota.as_ref().and_then(|q| q.plan_type.clone()),
                created_at: now_str.clone(),
                last_used_at: if is_first { Some(now_str) } else { None },
                is_active: is_first,
                quota,
            };

            existing.push(profile.clone());
            storage::save_accounts(provider, &existing)?;
            Ok(profile)
        }
        "chatgpt" => {
            // Support both auth.json and raw JWT / access token string
            let val_result: Result<serde_json::Value, _> = serde_json::from_str(raw_trimmed);
            let val = match val_result {
                Ok(v) => v,
                Err(_) if !raw_trimmed.starts_with('{') => {
                    serde_json::json!({
                        "tokens": {
                            "access_token": raw_trimmed,
                            "id_token": raw_trimmed
                        }
                    })
                }
                Err(e) => return Err(format!("Invalid JSON format for ChatGPT auth.json: {}", e)),
            };

            let serialized = serde_json::to_string_pretty(&val).map_err(|e| e.to_string())?;
            let identity = chatgpt::parse_identity_from_auth(&serialized);

            storage::save_account_data(provider, &account_id, "auth.json", serialized.as_bytes())?;
            let quota = chatgpt::query_quota(&serialized, &account_id, proxy).await.ok();

            let profile = AccountProfile {
                id: account_id.clone(),
                provider: provider.into(),
                name,
                email: identity.email.clone(),
                account_id: identity.account_id.clone().or(identity.email.clone()),
                plan_type: identity.plan_type.clone().or_else(|| quota.as_ref().and_then(|q| q.plan_type.clone())),
                created_at: now_str.clone(),
                last_used_at: if is_first { Some(now_str) } else { None },
                is_active: is_first,
                quota,
            };

            existing.push(profile.clone());
            storage::save_accounts(provider, &existing)?;
            Ok(profile)
        }
        "claude" => {
            // Claude requires config_auth.json or raw token cache
            let val_result: Result<serde_json::Value, _> = serde_json::from_str(raw_trimmed);
            let val = match val_result {
                Ok(v) => v,
                Err(_) if !raw_trimmed.starts_with('{') => {
                    serde_json::json!({
                        "oauth:tokenCacheV2": raw_trimmed
                    })
                }
                Err(e) => return Err(format!("Invalid JSON format for Claude config: {}", e)),
            };

            let serialized = serde_json::to_string_pretty(&val).map_err(|e| e.to_string())?;
            storage::save_account_data(provider, &account_id, "config_auth.json", serialized.as_bytes())?;

            let mut quota = None;
            if let Some(claude_dir) = claude::find_claude_data_dir() {
                quota = claude::query_quota(&claude_dir, &serialized, &account_id, proxy).await.ok();
            }

            let profile = AccountProfile {
                id: account_id.clone(),
                provider: provider.into(),
                name,
                email: quota.as_ref().and_then(|q| q.email.clone()),
                account_id: val.get("lastKnownAccountUuid").and_then(|v| v.as_str()).map(|s| s.to_string()),
                plan_type: quota.as_ref().and_then(|q| q.plan_type.clone()),
                created_at: now_str.clone(),
                last_used_at: if is_first { Some(now_str) } else { None },
                is_active: is_first,
                quota,
            };

            existing.push(profile.clone());
            storage::save_accounts(provider, &existing)?;
            Ok(profile)
        }
        "factory" => {
            let bundle = factory::create_bundle_from_raw(raw_trimmed)?;
            let identity = factory::parse_identity(&bundle.decrypted_json);

            factory::save_bundle_to_storage(provider, &account_id, &bundle)?;
            let quota = factory::query_quota(&bundle, &account_id, proxy).await.ok();

            let profile = AccountProfile {
                id: account_id.clone(),
                provider: provider.into(),
                name,
                email: identity.email.clone(),
                account_id: identity.subject.clone().or_else(|| identity.email.clone()),
                plan_type: identity.plan_type.clone().or_else(|| quota.as_ref().and_then(|q| q.plan_type.clone())),
                created_at: now_str.clone(),
                last_used_at: if is_first { Some(now_str) } else { None },
                is_active: is_first,
                quota,
            };

            existing.push(profile.clone());
            storage::save_accounts(provider, &existing)?;
            Ok(profile)
        }
        _ => Err("Unsupported provider".into()),
    }
}

pub async fn switch_account(
    app_id: &str,
    account_id: &str,
    pids_to_restart: Vec<u32>,
    target_spec: Option<&TargetAppSpec>,
    proxy: Option<&ProxyConfig>,
) -> Result<AccountSwitchResult, String> {
    let provider = normalize_provider(app_id)
        .ok_or_else(|| format!("App '{}' is not supported for account switching", app_id))?;

    let mut accounts = storage::load_accounts(provider)?;
    let target_idx = accounts
        .iter()
        .position(|a| a.id == account_id)
        .ok_or_else(|| format!("Target account '{}' not found", account_id))?;

    let had_running_processes = !pids_to_restart.is_empty();

    // 1. Stop running processes safely before swapping credentials
    if had_running_processes {
        let stopped = tokio::task::spawn_blocking(move || {
            ensure_stopped_before_relaunch(&pids_to_restart, true)
        })
        .await
        .map_err(|e| format!("Failed to stop running application: {}", e))?;

        if !stopped {
            return Err("无法关闭当前运行的应用进程，请先退出应用后再切换账号。".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    // 2. Perform credential swap
    match provider {
        "antigravity" => {
            // Save active credential of previously active account if one exists
            if let Some(active_account) = accounts.iter().find(|a| a.is_active && a.id != account_id) {
                if let Ok(active_payload) = antigravity::read_current_credential_payload() {
                    let _ = storage::save_account_data(provider, &active_account.id, "credential.json", active_payload.as_bytes());
                }
            }

            let data = storage::read_account_data(provider, account_id, "credential.json")?;
            let payload = String::from_utf8(data)
                .map_err(|e| format!("Invalid utf-8 in saved credential: {}", e))?;

            antigravity::write_credential_payload(&payload)?;
        }
        "chatgpt" => {
            // Save active auth to currently active account if one exists
            if let Some(active_account) = accounts.iter().find(|a| a.is_active && a.id != account_id) {
                if let Ok(active_auth) = chatgpt::read_current_auth_json() {
                    let _ = storage::save_account_data(provider, &active_account.id, "auth.json", active_auth.as_bytes());
                }
            }

            let target_data = storage::read_account_data(provider, account_id, "auth.json")?;
            let target_auth = String::from_utf8(target_data)
                .map_err(|e| format!("Invalid utf-8 in saved auth.json: {}", e))?;

            chatgpt::write_auth_json(&target_auth)?;
        }
        "claude" => {
            let claude_dir = claude::find_claude_data_dir()
                .ok_or_else(|| "Could not locate Claude data directory".to_string())?;

            // Save active state of previously active account
            if let Some(active_account) = accounts.iter().find(|a| a.is_active && a.id != account_id) {
                let cfg_path = claude_dir.join("config.json");
                if let Ok(cfg_raw) = std::fs::read_to_string(&cfg_path) {
                    if let Ok(cfg_val) = serde_json::from_str::<serde_json::Value>(&cfg_raw) {
                        let mut auth_keys = serde_json::Map::new();
                        for k in &["oauth:tokenCache", "oauth:tokenCacheV2", "lastKnownAccountUuid"] {
                            if let Some(v) = cfg_val.get(*k) {
                                auth_keys.insert(k.to_string(), v.clone());
                            }
                        }
                        if let Ok(ser) = serde_json::to_string_pretty(&auth_keys) {
                            let _ = storage::save_account_data(provider, &active_account.id, "config_auth.json", ser.as_bytes());
                        }
                    }
                }
                for f in claude::get_claude_cookie_files(&claude_dir) {
                    if f.is_file() {
                        if let Ok(bytes) = std::fs::read(&f) {
                            if let Some(fname) = f.file_name().and_then(|n| n.to_str()) {
                                let _ = storage::save_account_data(provider, &active_account.id, fname, &bytes);
                            }
                        }
                    }
                }
            }

            // Restore target auth config
            let target_auth_data = storage::read_account_data(provider, account_id, "config_auth.json")?;
            let target_auth_val: serde_json::Value = serde_json::from_slice(&target_auth_data)
                .map_err(|e| format!("Failed to parse target Claude config: {}", e))?;

            let cfg_path = claude_dir.join("config.json");
            let mut main_cfg: serde_json::Value = if cfg_path.is_file() {
                let raw = std::fs::read_to_string(&cfg_path).unwrap_or_default();
                serde_json::from_str(&raw).unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };

            if let Some(main_obj) = main_cfg.as_object_mut() {
                if let Some(target_obj) = target_auth_val.as_object() {
                    for (k, v) in target_obj {
                        main_obj.insert(k.clone(), v.clone());
                    }
                }
            }

            let new_cfg_str = serde_json::to_string_pretty(&main_cfg).map_err(|e| e.to_string())?;
            std::fs::write(&cfg_path, new_cfg_str).map_err(|e| format!("Failed to write Claude config.json: {}", e))?;

            // Restore cookies (including SQLite WAL files)
            let acct_dir = storage::get_account_dir(provider, account_id);
            for cookie_name in &["Cookies", "Cookies-journal", "Cookies-wal", "Cookies-shm"] {
                let saved_file = acct_dir.join(cookie_name);
                if saved_file.is_file() {
                    for target_dir in &[claude_dir.join("Network"), claude_dir.clone()] {
                        if target_dir.is_dir() {
                            let _ = std::fs::copy(&saved_file, target_dir.join(cookie_name));
                        }
                    }
                }
            }
        }
        "factory" => {
            // Save active files of previously active account if one exists
            if let Some(active_account) = accounts.iter().find(|a| a.is_active && a.id != account_id) {
                if let Ok(live_bundle) = factory::read_live_auth_bundle() {
                    let _ = factory::save_bundle_to_storage(provider, &active_account.id, &live_bundle);
                }
            }

            // Restore target account files to ~/.factory
            factory::restore_bundle_to_live(provider, account_id)?;
        }
        _ => return Err("Unsupported provider".into()),
    }

    // 3. Update accounts list state
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    for a in &mut accounts {
        a.is_active = a.id == account_id;
    }
    accounts[target_idx].last_used_at = Some(now_str);
    storage::save_accounts(provider, &accounts)?;

    // 4. Restart target app if it was running and configuration was passed
    let mut restarted = false;
    if had_running_processes {
        if let (Some(target), Some(p)) = (target_spec, proxy) {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            if launch_target(target, p).is_ok() {
                restarted = true;
            }
        }
    }

    Ok(AccountSwitchResult {
        success: true,
        restarted_app: restarted,
        message: format!("账号已成功切换至 {}", accounts[target_idx].name),
    })
}

pub fn delete_account(app_id: &str, account_id: &str) -> Result<(), String> {
    let provider = normalize_provider(app_id)
        .ok_or_else(|| format!("App '{}' is not supported", app_id))?;

    let mut accounts = storage::load_accounts(provider)?;
    let pos = accounts
        .iter()
        .position(|a| a.id == account_id)
        .ok_or_else(|| format!("Account '{}' not found", account_id))?;

    if accounts[pos].is_active && accounts.len() > 1 {
        return Err("无法删除当前处于激活状态的账号。请先切换至其他账号后再删除。".into());
    }

    accounts.remove(pos);
    storage::save_accounts(provider, &accounts)?;
    let _ = storage::delete_account_files(provider, account_id);
    Ok(())
}

pub async fn refresh_quota(
    app_id: &str,
    account_id: Option<String>,
    proxy: Option<&ProxyConfig>,
) -> Result<AccountQuota, String> {
    let provider = normalize_provider(app_id)
        .ok_or_else(|| format!("App '{}' is not supported", app_id))?;

    let mut accounts = storage::load_accounts(provider)?;
    let target_idx = if let Some(ref aid) = account_id {
        accounts.iter().position(|a| a.id == *aid)
    } else {
        accounts.iter().position(|a| a.is_active).or_else(|| if !accounts.is_empty() { Some(0) } else { None })
    };

    let idx = target_idx.ok_or_else(|| "No account found to refresh quota".to_string())?;
    let target_id = accounts[idx].id.clone();

    let quota = match provider {
        "antigravity" => {
            let data = storage::read_account_data(provider, &target_id, "credential.json")?;
            let mut payload = String::from_utf8(data).map_err(|e| e.to_string())?;
            let q = antigravity::query_quota(&mut payload, &target_id, proxy).await?;
            let _ = storage::save_account_data(provider, &target_id, "credential.json", payload.as_bytes());
            q
        }
        "chatgpt" => {
            let data = storage::read_account_data(provider, &target_id, "auth.json")?;
            let auth_json = String::from_utf8(data).map_err(|e| e.to_string())?;
            chatgpt::query_quota(&auth_json, &target_id, proxy).await?
        }
        "claude" => {
            let claude_dir = claude::find_claude_data_dir()
                .ok_or_else(|| "Claude data directory not found".to_string())?;
            let data = storage::read_account_data(provider, &target_id, "config_auth.json")?;
            let auth_cfg = String::from_utf8(data).map_err(|e| e.to_string())?;
            claude::query_quota(&claude_dir, &auth_cfg, &target_id, proxy).await?
        }
        "factory" => {
            // If the target account is currently active, sync live files from ~/.factory back to storage
            // to capture any refreshed access tokens updated in the background by Factory desktop app
            if accounts[idx].is_active {
                if let Ok(live_bundle) = factory::read_live_auth_bundle() {
                    let _ = factory::save_bundle_to_storage(provider, &target_id, &live_bundle);
                }
            }
            let bundle = factory::load_bundle_from_storage(provider, &target_id)?;
            factory::query_quota(&bundle, &target_id, proxy).await?
        }
        _ => return Err("Unsupported provider".into()),
    };

    accounts[idx].quota = Some(quota.clone());
    if quota.plan_type.is_some() {
        accounts[idx].plan_type = quota.plan_type.clone();
    }
    if accounts[idx].email.is_none() && quota.email.is_some() {
        accounts[idx].email = quota.email.clone();
        if accounts[idx].account_id.is_none() {
            accounts[idx].account_id = quota.email.clone();
        }
        if accounts[idx].name == "Antigravity Account" {
            if let Some(ref em) = quota.email {
                accounts[idx].name = em.split('@').next().unwrap_or("Antigravity Account").to_string();
            }
        }
    }
    let _ = storage::save_accounts(provider, &accounts);

    Ok(quota)
}

pub fn prepare_new_claude_login() -> Result<String, String> {
    let claude_dir = claude::find_claude_data_dir()
        .ok_or_else(|| "Could not locate Claude data directory".to_string())?;

    let cfg_path = claude_dir.join("config.json");
    if cfg_path.is_file() {
        let raw = std::fs::read_to_string(&cfg_path).unwrap_or_default();
        if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(obj) = val.as_object_mut() {
                obj.remove("oauth:tokenCache");
                obj.remove("oauth:tokenCacheV2");
                obj.remove("lastKnownAccountUuid");
            }
            let cleaned = serde_json::to_string_pretty(&val).map_err(|e| e.to_string())?;
            std::fs::write(&cfg_path, cleaned).map_err(|e| e.to_string())?;
        }
    }

    for f in claude::get_claude_cookie_files(&claude_dir) {
        if f.is_file() {
            let _ = std::fs::remove_file(f);
        }
    }

    Ok("已重置 Claude 桌面端当前会话，您可以重新启动 Claude 登录新账号，登录完成后点击「从当前客户端导入」即可保存。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_provider() {
        assert_eq!(normalize_provider("antigravity-app"), Some("antigravity"));
        assert_eq!(normalize_provider("antigravity-ide"), Some("antigravity"));
        assert_eq!(normalize_provider("chatgpt"), Some("chatgpt"));
        assert_eq!(normalize_provider("claude"), Some("claude"));
        assert_eq!(normalize_provider("factory"), Some("factory"));
        assert_eq!(normalize_provider("factory-app"), Some("factory"));
        assert_eq!(normalize_provider("factory-desktop"), Some("factory"));
        assert_eq!(normalize_provider("droid"), Some("factory"));
        assert_eq!(normalize_provider("unknown_app"), None);
    }
}

