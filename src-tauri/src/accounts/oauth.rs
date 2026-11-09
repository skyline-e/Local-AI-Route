use super::models::AccountProfile;
use super::{antigravity, chatgpt, claude, crypto, factory, storage};
use crate::core::models::ProxyConfig;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use uuid::Uuid;

const ANTIGRAVITY_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const ANTIGRAVITY_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const ANTIGRAVITY_SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs openid";

const CHATGPT_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CHATGPT_AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const CHATGPT_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CHATGPT_SCOPES: &str = "openid profile email offline_access";
const CHATGPT_DEFAULT_PORT: u16 = 1455;

const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_AUTH_URL: &str = "https://claude.ai/oauth/authorize";
const CLAUDE_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const CLAUDE_REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
const CLAUDE_SCOPES: &str = "org:create_api_key user:profile user:inference";

const WORKOS_CLIENT_ID_PROD: &str = "client_01HNM792M5G5G1A2THWPXKFMXB";
const WORKOS_DEVICE_AUTH_URL: &str = "https://api.workos.com/user_management/authorize/device";
const WORKOS_AUTHENTICATE_URL: &str = "https://api.workos.com/user_management/authenticate";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthStartInfo {
    pub session_id: String,
    pub provider: String,
    pub auth_url: String,
    pub user_code: Option<String>,
    pub redirect_uri: Option<String>,
    pub mode: String, // "local_callback" | "device_code" | "code_paste"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthPollResponse {
    pub status: String, // "pending" | "completed" | "failed" | "cancelled"
    pub account: Option<AccountProfile>,
    pub error: Option<String>,
}

#[derive(Clone)]
struct OAuthSession {
    provider: String,
    custom_name: Option<String>,
    state: String,
    code_verifier: String,
    redirect_uri: String,
    proxy: Option<ProxyConfig>,
    status: String, // "pending" | "exchanging" | "completed" | "failed" | "cancelled"
    account: Option<AccountProfile>,
    error: Option<String>,
    created_at: std::time::Instant,
    cancel_tx: watch::Sender<bool>,
}

fn sessions_store() -> &'static Mutex<HashMap<String, Arc<Mutex<OAuthSession>>>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Arc<Mutex<OAuthSession>>>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_unpoisoned<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn cleanup_stale_sessions(map: &mut HashMap<String, Arc<Mutex<OAuthSession>>>) {
    let max_age = std::time::Duration::from_secs(900);
    map.retain(|_, sess_arc| {
        let sess = lock_unpoisoned(sess_arc);
        let is_terminal = matches!(sess.status.as_str(), "completed" | "failed" | "cancelled");
        let keep = !is_terminal && sess.created_at.elapsed() < max_age;
        if !keep {
            let _ = sess.cancel_tx.send(true);
        }
        keep
    });
}

fn clean_custom_name(custom_name: Option<String>) -> Option<String> {
    custom_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn update_saved_account_name(provider: &str, account_id: &str, new_name: &str) {
    if let Ok(mut list) = storage::load_accounts(provider) {
        if let Some(acct) = list.iter_mut().find(|a| a.id == account_id) {
            if acct.name != new_name {
                acct.name = new_name.to_string();
                let _ = storage::save_accounts(provider, &list);
            }
        }
    }
}

pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

pub fn url_decode(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let raw = input.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'+' {
            bytes.push(b' ');
            i += 1;
        } else if raw[i] == b'%' && i + 2 < raw.len() {
            if let Ok(hex_str) = std::str::from_utf8(&raw[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                    bytes.push(val);
                    i += 3;
                    continue;
                }
            }
            bytes.push(raw[i]);
            i += 1;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

pub fn parse_query_params(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let trimmed = query.trim_start_matches('?');
    for pair in trimmed.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut parts = pair.splitn(2, '=');
        let k = url_decode(parts.next().unwrap_or(""));
        let v = url_decode(parts.next().unwrap_or(""));
        if !k.is_empty() {
            map.insert(k, v);
        }
    }
    map
}

/// Extracts `(code, state)` from a full callback URL, query string, `code#state` string, or raw code.
pub fn extract_code_and_state(raw_input: &str) -> Result<(String, Option<String>), String> {
    let trimmed = raw_input.trim();
    if trimmed.is_empty() {
        return Err("请输入有效的授权回调链接或授权码".into());
    }

    // 1. Check if it's a URL or query string containing `code=` or `error=`
    if trimmed.contains("code=")
        || trimmed.contains("error=")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        let query_part = if let Some(q_idx) = trimmed.find('?') {
            &trimmed[q_idx + 1..]
        } else {
            trimmed
        };
        let (query_clean, fragment_opt) = match query_part.split_once('#') {
            Some((q, f)) => (q, Some(f)),
            None => (query_part, None),
        };
        let params = parse_query_params(query_clean);
        if let Some(err) = params.get("error") {
            let desc = params
                .get("error_description")
                .cloned()
                .unwrap_or_else(|| err.clone());
            return Err(format!("OAuth 授权返回错误: {}", desc));
        }
        if let Some(code) = params.get("code") {
            if !code.trim().is_empty() {
                let state = params.get("state").cloned().or_else(|| {
                    fragment_opt
                        .map(|f| f.trim().to_string())
                        .filter(|s| !s.is_empty())
                });
                return Ok((code.trim().to_string(), state));
            }
        }
    }

    // 2. Check Anthropic `code#state` format
    if let Some((code_part, state_part)) = trimmed.split_once('#') {
        let c = code_part.trim();
        let s = state_part.trim();
        if !c.is_empty() {
            return Ok((
                c.to_string(),
                if s.is_empty() { None } else { Some(s.to_string()) },
            ));
        }
    }

    // 3. Raw authorization code
    Ok((trimmed.to_string(), None))
}

pub fn open_url_in_browser(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if !trimmed.starts_with("https://") && !trimmed.starts_with("http://") {
        return Err("Only http/https URLs are allowed".into());
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        std::process::Command::new("rundll32.exe")
            .arg("url.dll,FileProtocolHandler")
            .arg(trimmed)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(trimmed)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(trimmed)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {}", e))?;
    }

    Ok(())
}

async fn bind_chatgpt_callback_listener() -> Result<(TcpListener, u16), String> {
    if let Ok(listener) = TcpListener::bind(("127.0.0.1", CHATGPT_DEFAULT_PORT)).await {
        return Ok((listener, CHATGPT_DEFAULT_PORT));
    }

    // Attempt to gracefully close any previous stale login server on port 1455
    if let Ok(mut stream) =
        tokio::net::TcpStream::connect(("127.0.0.1", CHATGPT_DEFAULT_PORT)).await
    {
        let _ = stream
            .write_all(b"GET /cancel HTTP/1.1\r\nHost: 127.0.0.1:1455\r\nConnection: close\r\n\r\n")
            .await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;

    if let Ok(listener) = TcpListener::bind(("127.0.0.1", CHATGPT_DEFAULT_PORT)).await {
        return Ok((listener, CHATGPT_DEFAULT_PORT));
    }

    // Fallback to OS-assigned free port
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("Failed to bind local OAuth callback port: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to read local callback port: {}", e))?
        .port();
    Ok((listener, port))
}

pub async fn start_oauth_login(
    app_id: &str,
    custom_name: Option<String>,
    open_browser: bool,
    proxy: Option<ProxyConfig>,
) -> Result<OAuthStartInfo, String> {
    let provider = super::normalize_provider(app_id)
        .ok_or_else(|| format!("App '{}' does not support OAuth login", app_id))?;

    let session_id = Uuid::new_v4().to_string();
    let (verifier, challenge) = crypto::generate_pkce_pair();
    let state = crypto::generate_random_state();
    let (cancel_tx, cancel_rx) = watch::channel(false);

    let custom_name_clean = clean_custom_name(custom_name);

    {
        let mut map = lock_unpoisoned(sessions_store());
        cleanup_stale_sessions(&mut map);
    }

    match provider {
        "antigravity" => {
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|e| format!("Failed to start local OAuth listener: {}", e))?;
            let port = listener
                .local_addr()
                .map_err(|e| format!("Failed to read local listener port: {}", e))?
                .port();

            let redirect_uri = format!("http://127.0.0.1:{}/oauth-callback", port);
            let client_id = antigravity::get_oauth_client_id();
            let auth_url = format!(
                "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&state={}&code_challenge={}&code_challenge_method=S256",
                ANTIGRAVITY_AUTH_URL,
                url_encode(&client_id),
                url_encode(&redirect_uri),
                url_encode(ANTIGRAVITY_SCOPES),
                url_encode(&state),
                url_encode(&challenge)
            );

            let session = Arc::new(Mutex::new(OAuthSession {
                provider: provider.into(),
                custom_name: custom_name_clean,
                state: state.clone(),
                code_verifier: verifier,
                redirect_uri: redirect_uri.clone(),
                proxy: proxy.clone(),
                status: "pending".into(),
                account: None,
                error: None,
                created_at: std::time::Instant::now(),
                cancel_tx,
            }));

            lock_unpoisoned(sessions_store()).insert(session_id.clone(), session.clone());
            spawn_local_callback_listener(listener, session, cancel_rx);

            if open_browser {
                let _ = open_url_in_browser(&auth_url);
            }

            Ok(OAuthStartInfo {
                session_id,
                provider: provider.into(),
                auth_url,
                user_code: None,
                redirect_uri: Some(redirect_uri),
                mode: "local_callback".into(),
            })
        }
        "chatgpt" => {
            let (listener, port) = bind_chatgpt_callback_listener().await?;
            let redirect_uri = format!("http://localhost:{}/auth/callback", port);
            let auth_url = format!(
                "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&id_token_add_organizations=true&codex_cli_simplified_flow=true&state={}",
                CHATGPT_AUTH_URL,
                url_encode(CHATGPT_CLIENT_ID),
                url_encode(&redirect_uri),
                url_encode(CHATGPT_SCOPES),
                url_encode(&challenge),
                url_encode(&state)
            );

            let session = Arc::new(Mutex::new(OAuthSession {
                provider: provider.into(),
                custom_name: custom_name_clean,
                state: state.clone(),
                code_verifier: verifier,
                redirect_uri: redirect_uri.clone(),
                proxy: proxy.clone(),
                status: "pending".into(),
                account: None,
                error: None,
                created_at: std::time::Instant::now(),
                cancel_tx,
            }));

            lock_unpoisoned(sessions_store()).insert(session_id.clone(), session.clone());
            spawn_local_callback_listener(listener, session, cancel_rx);

            if open_browser {
                let _ = open_url_in_browser(&auth_url);
            }

            Ok(OAuthStartInfo {
                session_id,
                provider: provider.into(),
                auth_url,
                user_code: None,
                redirect_uri: Some(redirect_uri),
                mode: "local_callback".into(),
            })
        }
        "claude" => {
            let redirect_uri = CLAUDE_REDIRECT_URI.to_string();
            let auth_url = format!(
                "{}?code=true&client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
                CLAUDE_AUTH_URL,
                url_encode(CLAUDE_CLIENT_ID),
                url_encode(&redirect_uri),
                url_encode(CLAUDE_SCOPES),
                url_encode(&challenge),
                url_encode(&state)
            );

            let session = Arc::new(Mutex::new(OAuthSession {
                provider: provider.into(),
                custom_name: custom_name_clean,
                state,
                code_verifier: verifier,
                redirect_uri: redirect_uri.clone(),
                proxy,
                status: "pending".into(),
                account: None,
                error: None,
                created_at: std::time::Instant::now(),
                cancel_tx,
            }));

            lock_unpoisoned(sessions_store()).insert(session_id.clone(), session);

            if open_browser {
                let _ = open_url_in_browser(&auth_url);
            }

            Ok(OAuthStartInfo {
                session_id,
                provider: provider.into(),
                auth_url,
                user_code: None,
                redirect_uri: Some(redirect_uri),
                mode: "code_paste".into(),
            })
        }
        "factory" => {
            let client = super::build_http_client(proxy.as_ref(), 12);
            let resp = client
                .post(WORKOS_DEVICE_AUTH_URL)
                .header("User-Agent", "Factory Desktop")
                .form(&[("client_id", WORKOS_CLIENT_ID_PROD)])
                .send()
                .await
                .map_err(|e| format!("请求 Factory (WorkOS) 设备授权失败: {}", e))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Factory 设备授权接口返回错误 ({}): {}", status, body));
            }

            let data: Value = resp
                .json()
                .await
                .map_err(|e| format!("解析 Factory 设备授权响应失败: {}", e))?;

            let device_code = data
                .get("device_code")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "WorkOS 响应缺少 device_code".to_string())?
                .to_string();
            let user_code = data
                .get("user_code")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let verification_uri = data
                .get("verification_uri_complete")
                .or_else(|| data.get("verification_uri"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "WorkOS 响应缺少 verification_uri".to_string())?
                .to_string();
            let interval_secs = data.get("interval").and_then(|v| v.as_u64()).unwrap_or(5).max(2);
            let expires_in_secs = data.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(600);

            let session = Arc::new(Mutex::new(OAuthSession {
                provider: provider.into(),
                custom_name: custom_name_clean,
                state,
                code_verifier: verifier,
                redirect_uri: String::new(),
                proxy: proxy.clone(),
                status: "pending".into(),
                account: None,
                error: None,
                created_at: std::time::Instant::now(),
                cancel_tx,
            }));

            lock_unpoisoned(sessions_store()).insert(session_id.clone(), session.clone());
            spawn_factory_device_polling(
                device_code,
                interval_secs,
                expires_in_secs,
                session,
                cancel_rx,
            );

            if open_browser {
                let _ = open_url_in_browser(&verification_uri);
            }

            Ok(OAuthStartInfo {
                session_id,
                provider: provider.into(),
                auth_url: verification_uri,
                user_code: if user_code.is_empty() { None } else { Some(user_code) },
                redirect_uri: None,
                mode: "device_code".into(),
            })
        }
        _ => Err("Unsupported provider for OAuth login".into()),
    }
}

pub fn poll_oauth_login(
    session_id: &str,
    custom_name: Option<String>,
) -> Result<OAuthPollResponse, String> {
    let custom_name_clean = clean_custom_name(custom_name);

    let session_arc = {
        let map = lock_unpoisoned(sessions_store());
        map.get(session_id)
            .cloned()
            .ok_or_else(|| "OAuth 会话不存在或已结束".to_string())?
    };

    let (resp, should_remove) = {
        let mut sess = lock_unpoisoned(&session_arc);
        if sess.account.is_none() {
            sess.custom_name = custom_name_clean.clone();
        } else if let Some(ref new_label) = custom_name_clean {
            sess.custom_name = Some(new_label.clone());
            let provider = sess.provider.clone();
            if let Some(ref mut acct) = sess.account {
                if acct.name != *new_label {
                    acct.name = new_label.clone();
                    update_saved_account_name(&provider, &acct.id, new_label);
                }
            }
        }

        let public_status = if sess.status == "exchanging" {
            "pending".to_string()
        } else {
            sess.status.clone()
        };
        let is_terminal = matches!(public_status.as_str(), "completed" | "failed" | "cancelled");

        (
            OAuthPollResponse {
                status: public_status,
                account: sess.account.clone(),
                error: sess.error.clone(),
            },
            is_terminal,
        )
    };

    if should_remove {
        lock_unpoisoned(sessions_store()).remove(session_id);
    }

    Ok(resp)
}

pub fn cancel_oauth_login(session_id: &str) -> Result<(), String> {
    let session_arc_opt = {
        let mut map = lock_unpoisoned(sessions_store());
        map.remove(session_id)
    };
    if let Some(session_arc) = session_arc_opt {
        let mut sess = lock_unpoisoned(&session_arc);
        sess.status = "cancelled".into();
        let _ = sess.cancel_tx.send(true);
    }
    Ok(())
}

pub async fn submit_oauth_callback(
    session_id: &str,
    callback_input: &str,
    custom_name: Option<String>,
) -> Result<AccountProfile, String> {
    let session_arc = {
        let map = lock_unpoisoned(sessions_store());
        map.get(session_id)
            .cloned()
            .ok_or_else(|| "OAuth 会话不存在或已过期，请重新发起 OAuth 登录".to_string())?
    };

    let custom_name_override = clean_custom_name(custom_name);

    let (provider, effective_name, expected_state, code_verifier, redirect_uri, proxy) = {
        let mut sess = lock_unpoisoned(&session_arc);
        if sess.status == "completed" {
            let provider = sess.provider.clone();
            if let Some(ref mut acct) = sess.account {
                if let Some(ref new_label) = custom_name_override {
                    if acct.name != *new_label {
                        acct.name = new_label.clone();
                        update_saved_account_name(&provider, &acct.id, new_label);
                    }
                }
                let cloned = acct.clone();
                drop(sess);
                lock_unpoisoned(sessions_store()).remove(session_id);
                return Ok(cloned);
            }
        }
        if sess.status == "exchanging" {
            return Err("正在处理浏览器自动回调，请稍候...".into());
        }
        if sess.status == "cancelled" {
            return Err("当前 OAuth 会话已取消，请重新发起授权".into());
        }
        sess.status = "exchanging".into();
        let _ = sess.cancel_tx.send(true);
        sess.custom_name = custom_name_override.clone();
        (
            sess.provider.clone(),
            sess.custom_name.clone(),
            sess.state.clone(),
            sess.code_verifier.clone(),
            sess.redirect_uri.clone(),
            sess.proxy.clone(),
        )
    };

    let exchange_res = match provider.as_str() {
        "antigravity" => match extract_code_and_state(callback_input) {
            Ok((code, state_opt)) => {
                if let Some(st) = state_opt {
                    if !expected_state.is_empty() && st != expected_state {
                        let mut sess = lock_unpoisoned(&session_arc);
                        sess.status = "pending".into();
                        return Err("OAuth state 校验不匹配，请使用同一次发起的授权链接重试".into());
                    }
                }
                exchange_and_save_antigravity(
                    &code,
                    &code_verifier,
                    &redirect_uri,
                    effective_name,
                    proxy.as_ref(),
                )
                .await
            }
            Err(e) => Err(e),
        },
        "chatgpt" => match extract_code_and_state(callback_input) {
            Ok((code, state_opt)) => {
                if let Some(st) = state_opt {
                    if !expected_state.is_empty() && st != expected_state {
                        let mut sess = lock_unpoisoned(&session_arc);
                        sess.status = "pending".into();
                        return Err("OAuth state 校验不匹配，请使用同一次发起的授权链接重试".into());
                    }
                }
                exchange_and_save_chatgpt(
                    &code,
                    &code_verifier,
                    &redirect_uri,
                    effective_name,
                    proxy.as_ref(),
                )
                .await
            }
            Err(e) => Err(e),
        },
        "claude" => match extract_code_and_state(callback_input) {
            Ok((code, state_opt)) => {
                if let Some(ref st) = state_opt {
                    if !expected_state.is_empty() && st != &expected_state {
                        let mut sess = lock_unpoisoned(&session_arc);
                        sess.status = "pending".into();
                        return Err("OAuth state 校验不匹配，请使用同一次发起的授权链接重试".into());
                    }
                }
                let state_to_send = state_opt.unwrap_or_else(|| expected_state.clone());
                exchange_and_save_claude(
                    &code,
                    &state_to_send,
                    &code_verifier,
                    &redirect_uri,
                    effective_name,
                    proxy.as_ref(),
                )
                .await
            }
            Err(e) => Err(e),
        },
        "factory" => {
            let fallback_name = effective_name.unwrap_or_else(|| "Factory Account".into());
            super::add_account_manual(
                "factory",
                fallback_name,
                callback_input.trim().to_string(),
                proxy.as_ref(),
            )
            .await
        }
        _ => Err("Unsupported provider".into()),
    };

    match exchange_res {
        Ok(profile) => {
            {
                let mut sess = lock_unpoisoned(&session_arc);
                sess.status = "completed".into();
                sess.account = Some(profile.clone());
                sess.error = None;
            }
            lock_unpoisoned(sessions_store()).remove(session_id);
            Ok(profile)
        }
        Err(err) => {
            // Allow user to correct pasted input and retry
            let mut sess = lock_unpoisoned(&session_arc);
            sess.status = "pending".into();
            sess.error = Some(err.clone());
            Err(err)
        }
    }
}

fn spawn_local_callback_listener(
    listener: TcpListener,
    session: Arc<Mutex<OAuthSession>>,
    mut cancel_rx: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);

        loop {
            let accept_res = tokio::select! {
                _ = cancel_rx.changed() => {
                    if *cancel_rx.borrow() {
                        return;
                    }
                    continue;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    let mut sess = lock_unpoisoned(&session);
                    if sess.status == "pending" {
                        sess.status = "failed".into();
                        sess.error = Some("等待浏览器 OAuth 回调超时（10分钟）".into());
                    }
                    return;
                }
                res = listener.accept() => res,
            };

            let Ok((mut stream, _)) = accept_res else {
                continue;
            };

            let mut buf = [0u8; 8192];
            let Ok(Ok(n)) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                stream.read(&mut buf),
            )
            .await
            else {
                continue;
            };
            if n == 0 {
                continue;
            }

            let req_str = String::from_utf8_lossy(&buf[..n]);
            let first_line = req_str.lines().next().unwrap_or("");
            let path = first_line.split_whitespace().nth(1).unwrap_or("");

            if path.starts_with("/cancel") {
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
                    .await;
                let mut sess = lock_unpoisoned(&session);
                if sess.status == "pending" {
                    sess.status = "cancelled".into();
                }
                return;
            }

            if path.starts_with("/favicon.ico") || (!path.contains("code=") && !path.contains("error=")) {
                let _ = stream
                    .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                    .await;
                continue;
            }

            match extract_code_and_state(path) {
                Ok((code, state_opt)) => {
                    let check_res = {
                        let mut sess = lock_unpoisoned(&session);
                        if sess.status != "pending" {
                            None
                        } else if state_opt
                            .as_ref()
                            .map(|st| !sess.state.is_empty() && st != &sess.state)
                            .unwrap_or(false)
                        {
                            sess.status = "failed".into();
                            sess.error = Some("OAuth state 校验不匹配".into());
                            Some(Err(()))
                        } else {
                            sess.status = "exchanging".into();
                            Some(Ok((
                                sess.provider.clone(),
                                sess.code_verifier.clone(),
                                sess.redirect_uri.clone(),
                                sess.proxy.clone(),
                            )))
                        }
                    };

                    let (provider, code_verifier, redirect_uri, proxy) = match check_res {
                        None => return,
                        Some(Err(())) => {
                            let html = build_callback_html(false, "State 校验不匹配 / State mismatch");
                            let resp = format!(
                                "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                html.len(),
                                html
                            );
                            let _ = stream.write_all(resp.as_bytes()).await;
                            return;
                        }
                        Some(Ok(tuple)) => tuple,
                    };

                    let html = build_callback_html(
                        true,
                        "OAuth 授权已完成，正在同步账号与配额信息，您现在可以关闭此浏览器标签页返回 Local-AI-Route。",
                    );
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        html.len(),
                        html
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    let _ = stream.flush().await;
                    drop(stream);

                    let current_custom_name = {
                        let sess = lock_unpoisoned(&session);
                        sess.custom_name.clone()
                    };

                    let exchange_res = match provider.as_str() {
                        "antigravity" => {
                            exchange_and_save_antigravity(
                                &code,
                                &code_verifier,
                                &redirect_uri,
                                current_custom_name,
                                proxy.as_ref(),
                            )
                            .await
                        }
                        "chatgpt" => {
                            exchange_and_save_chatgpt(
                                &code,
                                &code_verifier,
                                &redirect_uri,
                                current_custom_name,
                                proxy.as_ref(),
                            )
                            .await
                        }
                        _ => Err("Unsupported callback provider".into()),
                    };

                    let mut sess = lock_unpoisoned(&session);
                    match exchange_res {
                        Ok(mut profile) => {
                            if let Some(ref latest_name) = sess.custom_name {
                                if profile.name != *latest_name {
                                    profile.name = latest_name.clone();
                                    update_saved_account_name(&sess.provider, &profile.id, latest_name);
                                }
                            }
                            sess.status = "completed".into();
                            sess.account = Some(profile);
                            sess.error = None;
                        }
                        Err(e) => {
                            sess.status = "failed".into();
                            sess.error = Some(e);
                        }
                    }
                    return;
                }
                Err(err_msg) => {
                    let html = build_callback_html(false, &err_msg);
                    let resp = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        html.len(),
                        html
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    let mut sess = lock_unpoisoned(&session);
                    if sess.status == "pending" {
                        sess.status = "failed".into();
                        sess.error = Some(err_msg);
                    }
                    return;
                }
            }
        }
    });
}

fn spawn_factory_device_polling(
    device_code: String,
    initial_interval_secs: u64,
    expires_in_secs: u64,
    session: Arc<Mutex<OAuthSession>>,
    mut cancel_rx: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(expires_in_secs);
        let mut interval_secs = initial_interval_secs;
        let proxy = {
            let sess = lock_unpoisoned(&session);
            sess.proxy.clone()
        };
        let client = super::build_http_client(proxy.as_ref(), 12);

        loop {
            tokio::select! {
                _ = cancel_rx.changed() => {
                    if *cancel_rx.borrow() {
                        return;
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    let mut sess = lock_unpoisoned(&session);
                    if sess.status == "pending" {
                        sess.status = "failed".into();
                        sess.error = Some("Factory 设备授权码已过期，请重新发起登录".into());
                    }
                    return;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(interval_secs)) => {}
            }

            let resp_res = client
                .post(WORKOS_AUTHENTICATE_URL)
                .header("User-Agent", "Factory Desktop")
                .form(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", device_code.as_str()),
                    ("client_id", WORKOS_CLIENT_ID_PROD),
                ])
                .send()
                .await;

            let Ok(resp) = resp_res else {
                continue;
            };

            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();

            if status.is_success() {
                let custom_name = {
                    let mut sess = lock_unpoisoned(&session);
                    if sess.status != "pending" {
                        return;
                    }
                    sess.status = "exchanging".into();
                    sess.custom_name.clone()
                };
                let save_res =
                    save_factory_oauth_response(&body_text, custom_name, proxy.as_ref()).await;
                let mut sess = lock_unpoisoned(&session);
                match save_res {
                    Ok(mut profile) => {
                        if let Some(ref latest_name) = sess.custom_name {
                            if profile.name != *latest_name {
                                profile.name = latest_name.clone();
                                update_saved_account_name(&sess.provider, &profile.id, latest_name);
                            }
                        }
                        sess.status = "completed".into();
                        sess.account = Some(profile);
                        sess.error = None;
                    }
                    Err(e) => {
                        sess.status = "failed".into();
                        sess.error = Some(e);
                    }
                }
                return;
            }

            if let Ok(err_val) = serde_json::from_str::<Value>(&body_text) {
                let err_code = err_val.get("error").and_then(|v| v.as_str()).unwrap_or("");
                match err_code {
                    "authorization_pending" => continue,
                    "slow_down" => {
                        interval_secs += 2;
                        continue;
                    }
                    "access_denied" | "expired_token" => {
                        let mut sess = lock_unpoisoned(&session);
                        if sess.status == "pending" {
                            sess.status = "failed".into();
                            sess.error = Some(format!("Factory 授权未通过或已过期 ({})", err_code));
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }
    });
}

async fn exchange_and_save_antigravity(
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    custom_name: Option<String>,
    proxy: Option<&ProxyConfig>,
) -> Result<AccountProfile, String> {
    let client = super::build_http_client(proxy, 15);
    let client_id = antigravity::get_oauth_client_id();
    let client_secret = antigravity::get_oauth_client_secret();

    let params = [
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("code", code),
        ("code_verifier", code_verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri),
    ];

    let resp = client
        .post(ANTIGRAVITY_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Google OAuth token 交换请求失败: {}", e))?;

    if !resp.status().is_success() {
        let err_body = resp.text().await.unwrap_or_default();
        return Err(format!("Google OAuth token 交换失败: {}", err_body));
    }

    let token_data: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 Google OAuth token 响应失败: {}", e))?;

    let access_token = token_data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Google OAuth 响应缺少 access_token".to_string())?;
    let mut refresh_token = token_data
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id_token = token_data
        .get("id_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let expires_in = token_data.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(3600);
    let expiry = (Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339();

    let mut payload_val = json!({
        "token": {
            "access_token": access_token,
            "refresh_token": refresh_token,
            "token_type": "Bearer",
            "expiry": expiry,
            "id_token": id_token,
        },
        "auth_method": "consumer"
    });

    let mut payload_str = serde_json::to_string(&payload_val).map_err(|e| e.to_string())?;
    let mut email = antigravity::parse_email_from_payload(&payload_str);

    if email.is_none() {
        if let Ok(u_resp) = client
            .get("https://www.googleapis.com/oauth2/v3/userinfo")
            .bearer_auth(access_token)
            .send()
            .await
        {
            if u_resp.status().is_success() {
                if let Ok(uinfo) = u_resp.json::<Value>().await {
                    if let Some(em) = uinfo.get("email").and_then(|e| e.as_str()) {
                        email = Some(em.to_string());
                    }
                }
            }
        }
    }

    let provider = "antigravity";
    let mut existing = storage::load_accounts(provider).unwrap_or_default();
    let is_first = existing.is_empty();
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let existing_pos = email
        .as_ref()
        .and_then(|em| existing.iter().position(|a| a.email.as_deref() == Some(em.as_str())));

    let target_id = if let Some(pos) = existing_pos {
        existing[pos].id.clone()
    } else {
        Uuid::new_v4().to_string()
    };

    // Preserve previous refresh_token if Google OAuth omitted it on re-authorization
    if refresh_token.is_empty() && existing_pos.is_some() {
        if let Ok(old_bytes) = storage::read_account_data(provider, &target_id, "credential.json") {
            if let Ok(old_val) = serde_json::from_slice::<Value>(&old_bytes) {
                if let Some(old_rt) = old_val
                    .pointer("/token/refresh_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    refresh_token = old_rt.to_string();
                    payload_val["token"]["refresh_token"] = Value::String(refresh_token);
                }
            }
        }
    }

    if let Some(ref em) = email {
        payload_val["id_token_email"] = Value::String(em.clone());
    }
    payload_str = serde_json::to_string(&payload_val).map_err(|e| e.to_string())?;

    storage::save_account_data(provider, &target_id, "credential.json", payload_str.as_bytes())?;
    let quota = antigravity::query_quota(&mut payload_str, &target_id, proxy).await.ok();
    let _ = storage::save_account_data(provider, &target_id, "credential.json", payload_str.as_bytes());

    if email.is_none() {
        email = quota.as_ref().and_then(|q| q.email.clone());
    }

    let display_name = custom_name.clone().unwrap_or_else(|| {
        email
            .as_ref()
            .and_then(|em| em.split('@').next())
            .unwrap_or("Antigravity Account")
            .to_string()
    });

    if let Some(pos) = existing_pos {
        if let Some(custom) = custom_name {
            existing[pos].name = custom;
        } else if existing[pos].name.is_empty() || existing[pos].name == "Antigravity Account" {
            existing[pos].name = display_name;
        }
        existing[pos].email = email.clone().or(existing[pos].email.clone());
        existing[pos].account_id = email.or(existing[pos].account_id.clone());
        if let Some(ref q) = quota {
            if q.plan_type.is_some() {
                existing[pos].plan_type = q.plan_type.clone();
            }
            existing[pos].quota = Some(q.clone());
        }
        storage::save_accounts(provider, &existing)?;
        return Ok(existing[pos].clone());
    }

    let profile = AccountProfile {
        id: target_id,
        provider: provider.into(),
        name: display_name,
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

async fn exchange_and_save_chatgpt(
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    custom_name: Option<String>,
    proxy: Option<&ProxyConfig>,
) -> Result<AccountProfile, String> {
    let client = super::build_http_client(proxy, 15);
    let params = [
        ("grant_type", "authorization_code"),
        ("client_id", CHATGPT_CLIENT_ID),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
    ];

    let resp = client
        .post(CHATGPT_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("OpenAI OAuth token 交换请求失败: {}", e))?;

    if !resp.status().is_success() {
        let err_body = resp.text().await.unwrap_or_default();
        return Err(format!("OpenAI OAuth token 交换失败: {}", err_body));
    }

    let token_data: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 OpenAI OAuth token 响应失败: {}", e))?;

    let access_token = token_data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "OpenAI OAuth 响应缺少 access_token".to_string())?;
    let id_token = token_data
        .get("id_token")
        .and_then(|v| v.as_str())
        .unwrap_or(access_token);
    let mut refresh_token = token_data
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut auth_val = json!({
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": id_token,
            "access_token": access_token,
            "refresh_token": refresh_token,
        },
        "last_refresh": Utc::now().to_rfc3339()
    });

    let temp_str = serde_json::to_string(&auth_val).unwrap_or_default();
    let identity = chatgpt::parse_identity_from_auth(&temp_str);
    if let Some(ref aid) = identity.account_id {
        auth_val["tokens"]["account_id"] = Value::String(aid.clone());
    }

    let provider = "chatgpt";
    let mut existing = storage::load_accounts(provider).unwrap_or_default();
    let is_first = existing.is_empty();
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let existing_pos = identity
        .email
        .as_ref()
        .and_then(|em| existing.iter().position(|a| a.email.as_deref() == Some(em.as_str())));

    let target_id = if let Some(pos) = existing_pos {
        existing[pos].id.clone()
    } else {
        Uuid::new_v4().to_string()
    };

    // Preserve previous refresh_token if omitted on re-authorization
    if refresh_token.is_empty() && existing_pos.is_some() {
        if let Ok(old_bytes) = storage::read_account_data(provider, &target_id, "auth.json") {
            if let Ok(old_val) = serde_json::from_slice::<Value>(&old_bytes) {
                if let Some(old_rt) = old_val
                    .pointer("/tokens/refresh_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    refresh_token = old_rt.to_string();
                    auth_val["tokens"]["refresh_token"] = Value::String(refresh_token);
                }
            }
        }
    }

    let serialized = serde_json::to_string_pretty(&auth_val).map_err(|e| e.to_string())?;

    storage::save_account_data(provider, &target_id, "auth.json", serialized.as_bytes())?;
    let quota = chatgpt::query_quota(&serialized, &target_id, proxy).await.ok();

    let display_name = custom_name.clone().unwrap_or_else(|| {
        identity
            .email
            .as_ref()
            .and_then(|em| em.split('@').next())
            .unwrap_or("ChatGPT Account")
            .to_string()
    });

    if let Some(pos) = existing_pos {
        if let Some(custom) = custom_name {
            existing[pos].name = custom;
        } else if existing[pos].name.is_empty() || existing[pos].name == "ChatGPT Account" {
            existing[pos].name = display_name;
        }
        existing[pos].email = identity.email.clone().or(existing[pos].email.clone());
        existing[pos].account_id = identity.account_id.clone().or(existing[pos].account_id.clone());
        if let Some(ref q) = quota {
            if q.plan_type.is_some() {
                existing[pos].plan_type = q.plan_type.clone();
            }
            existing[pos].quota = Some(q.clone());
        }
        storage::save_accounts(provider, &existing)?;
        return Ok(existing[pos].clone());
    }

    let profile = AccountProfile {
        id: target_id,
        provider: provider.into(),
        name: display_name,
        email: identity.email.clone(),
        account_id: identity.account_id.or(identity.email),
        plan_type: quota
            .as_ref()
            .and_then(|q| q.plan_type.clone())
            .or(identity.plan_type),
        created_at: now_str.clone(),
        last_used_at: if is_first { Some(now_str) } else { None },
        is_active: is_first,
        quota,
    };

    existing.push(profile.clone());
    storage::save_accounts(provider, &existing)?;
    Ok(profile)
}

async fn exchange_and_save_claude(
    code: &str,
    state: &str,
    code_verifier: &str,
    redirect_uri: &str,
    custom_name: Option<String>,
    proxy: Option<&ProxyConfig>,
) -> Result<AccountProfile, String> {
    let client = super::build_http_client(proxy, 15);
    let body = json!({
        "grant_type": "authorization_code",
        "client_id": CLAUDE_CLIENT_ID,
        "code": code,
        "state": state,
        "redirect_uri": redirect_uri,
        "code_verifier": code_verifier,
    });

    let resp = client
        .post(CLAUDE_TOKEN_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Claude OAuth token 交换请求失败: {}", e))?;

    if !resp.status().is_success() {
        let err_body = resp.text().await.unwrap_or_default();
        return Err(format!("Claude OAuth token 交换失败: {}", err_body));
    }

    let token_data: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 Claude OAuth token 响应失败: {}", e))?;

    let access_token = token_data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Claude OAuth 响应缺少 access_token".to_string())?;
    let mut refresh_token = token_data
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let expires_in = token_data.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(28800);
    let expires_at_ms = (Utc::now() + chrono::Duration::seconds(expires_in)).timestamp_millis();

    let account_uuid = token_data
        .pointer("/account/uuid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let mut email = token_data
        .pointer("/account/email_address")
        .or_else(|| token_data.pointer("/account/email"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let claude_dir_opt = claude::find_claude_data_dir();
    let token_cache_v2 = if let Some(ref c_dir) = claude_dir_opt {
        claude::encode_token_cache_v2(c_dir, access_token, expires_at_ms, account_uuid.as_deref())
            .unwrap_or_else(|_| access_token.to_string())
    } else {
        access_token.to_string()
    };

    let mut auth_obj = serde_json::Map::new();
    auth_obj.insert("oauth:tokenCacheV2".to_string(), Value::String(token_cache_v2));
    auth_obj.insert("oauth_access_token".to_string(), Value::String(access_token.to_string()));
    if !refresh_token.is_empty() {
        auth_obj.insert("oauth_refresh_token".to_string(), Value::String(refresh_token.clone()));
    }
    auth_obj.insert("oauth_expires_at".to_string(), Value::Number(expires_at_ms.into()));
    if let Some(ref uuid_str) = account_uuid {
        auth_obj.insert("lastKnownAccountUuid".to_string(), Value::String(uuid_str.clone()));
    }

    let mut serialized = serde_json::to_string_pretty(&auth_obj).map_err(|e| e.to_string())?;

    let provider = "claude";
    let mut existing = storage::load_accounts(provider).unwrap_or_default();
    let is_first = existing.is_empty();
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let dummy_dir = claude_dir_opt.unwrap_or_else(|| std::path::PathBuf::from("."));
    let initial_existing_pos = email
        .as_ref()
        .and_then(|em| existing.iter().position(|a| a.email.as_deref() == Some(em.as_str())));
    let temp_id = initial_existing_pos
        .map(|pos| existing[pos].id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let quota = claude::query_quota(&dummy_dir, &serialized, &temp_id, proxy).await.ok();

    if email.is_none() {
        email = quota.as_ref().and_then(|q| q.email.clone());
    }

    let existing_pos = initial_existing_pos.or_else(|| {
        email
            .as_ref()
            .and_then(|em| existing.iter().position(|a| a.email.as_deref() == Some(em.as_str())))
    });

    let target_id = if let Some(pos) = existing_pos {
        let existing_id = existing[pos].id.clone();
        if existing_id != temp_id {
            let _ = storage::delete_account_files(provider, &temp_id);
        }
        existing_id
    } else {
        temp_id
    };

    if refresh_token.is_empty() && existing_pos.is_some() {
        if let Ok(old_bytes) = storage::read_account_data(provider, &target_id, "config_auth.json") {
            if let Ok(old_val) = serde_json::from_slice::<Value>(&old_bytes) {
                if let Some(old_rt) = old_val
                    .get("oauth_refresh_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    refresh_token = old_rt.to_string();
                    auth_obj.insert("oauth_refresh_token".to_string(), Value::String(refresh_token));
                    serialized = serde_json::to_string_pretty(&auth_obj).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    storage::save_account_data(provider, &target_id, "config_auth.json", serialized.as_bytes())?;

    let display_name = custom_name.clone().unwrap_or_else(|| {
        email
            .as_ref()
            .and_then(|em| em.split('@').next())
            .unwrap_or("Claude Account")
            .to_string()
    });

    if let Some(pos) = existing_pos {
        if let Some(custom) = custom_name {
            existing[pos].name = custom;
        } else if existing[pos].name.is_empty() || existing[pos].name == "Claude Account" {
            existing[pos].name = display_name;
        }
        existing[pos].email = email.clone().or(existing[pos].email.clone());
        existing[pos].account_id = account_uuid.or(existing[pos].account_id.clone());
        if let Some(mut q) = quota {
            q.account_id = target_id;
            if q.plan_type.is_some() {
                existing[pos].plan_type = q.plan_type.clone();
            }
            existing[pos].quota = Some(q);
        }
        storage::save_accounts(provider, &existing)?;
        return Ok(existing[pos].clone());
    }

    let profile = AccountProfile {
        id: target_id,
        provider: provider.into(),
        name: display_name,
        email,
        account_id: account_uuid,
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

async fn save_factory_oauth_response(
    raw_json_str: &str,
    custom_name: Option<String>,
    proxy: Option<&ProxyConfig>,
) -> Result<AccountProfile, String> {
    let val: Value = serde_json::from_str(raw_json_str)
        .map_err(|e| format!("解析 Factory OAuth token JSON 失败: {}", e))?;

    let access_token = val
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Factory OAuth 响应缺少 access_token".to_string())?;
    let refresh_token = val
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let org_id = val
        .get("organization_id")
        .or_else(|| val.get("active_organization_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut payload_obj = json!({
        "access_token": access_token,
        "refresh_token": refresh_token,
    });
    if !org_id.is_empty() {
        payload_obj["active_organization_id"] = Value::String(org_id.to_string());
    }

    let mut bundle = factory::create_bundle_from_raw(&payload_obj.to_string())?;
    let identity = factory::parse_identity(&bundle.decrypted_json);

    let provider = "factory";
    let mut existing = storage::load_accounts(provider).unwrap_or_default();
    let is_first = existing.is_empty();
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let existing_pos = existing.iter().position(|a| {
        if let (Some(em1), Some(em2)) = (&a.email, &identity.email) {
            if em1.eq_ignore_ascii_case(em2) {
                return true;
            }
        }
        if let (Some(id1), Some(id2)) = (&a.account_id, &identity.subject) {
            if id1 == id2 {
                return true;
            }
        }
        false
    });

    let target_id = if let Some(pos) = existing_pos {
        existing[pos].id.clone()
    } else {
        Uuid::new_v4().to_string()
    };

    if refresh_token.is_empty() && existing_pos.is_some() {
        if let Ok(old_bundle) = factory::load_bundle_from_storage(provider, &target_id) {
            if let Ok(old_val) = serde_json::from_str::<Value>(&old_bundle.decrypted_json) {
                if let Some(old_rt) = old_val
                    .get("refresh_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    payload_obj["refresh_token"] = Value::String(old_rt.to_string());
                    if let Ok(rebuilt) = factory::create_bundle_from_raw(&payload_obj.to_string()) {
                        bundle = rebuilt;
                    }
                }
            }
        }
    }

    let quota = factory::query_quota(&mut bundle, &target_id, proxy).await.ok();
    let identity = factory::parse_identity(&bundle.decrypted_json);
    factory::save_bundle_to_storage(provider, &target_id, &bundle)?;

    let display_name = custom_name.clone().unwrap_or_else(|| {
        identity
            .email
            .as_ref()
            .and_then(|em| em.split('@').next())
            .unwrap_or("Factory Account")
            .to_string()
    });

    if let Some(pos) = existing_pos {
        if let Some(custom) = custom_name {
            existing[pos].name = custom;
        } else if existing[pos].name.is_empty() || existing[pos].name == "Factory Account" {
            existing[pos].name = display_name;
        }
        existing[pos].email = identity.email.clone().or(existing[pos].email.clone());
        existing[pos].account_id = identity.subject.clone().or(existing[pos].account_id.clone());
        if let Some(ref q) = quota {
            if q.plan_type.is_some() {
                existing[pos].plan_type = q.plan_type.clone();
            }
            existing[pos].quota = Some(q.clone());
        }
        storage::save_accounts(provider, &existing)?;
        return Ok(existing[pos].clone());
    }

    let profile = AccountProfile {
        id: target_id,
        provider: provider.into(),
        name: display_name,
        email: identity.email.clone(),
        account_id: identity.subject.or(identity.email),
        plan_type: quota
            .as_ref()
            .and_then(|q| q.plan_type.clone())
            .or(identity.plan_type),
        created_at: now_str.clone(),
        last_used_at: if is_first { Some(now_str) } else { None },
        is_active: is_first,
        quota,
    };

    existing.push(profile.clone());
    storage::save_accounts(provider, &existing)?;
    Ok(profile)
}

fn build_callback_html(success: bool, message: &str) -> String {
    let (title, color, badge) = if success {
        ("OAuth 授权成功 · Local-AI-Route", "#10b981", "✓ 授权成功")
    } else {
        ("OAuth 授权失败 · Local-AI-Route", "#ef4444", "✕ 授权失败")
    };
    let safe_message = escape_html(message);
    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>{title}</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #0f172a; color: #f8fafc; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }}
  .card {{ background: #1e293b; border: 1px solid #334155; border-radius: 16px; padding: 32px; max-width: 440px; text-align: center; box-shadow: 0 20px 25px -5px rgba(0,0,0,0.4); }}
  .badge {{ display: inline-block; padding: 6px 14px; border-radius: 9999px; font-weight: 700; font-size: 14px; color: #fff; background: {color}; margin-bottom: 16px; }}
  h1 {{ font-size: 18px; margin: 0 0 12px; }}
  p {{ font-size: 13px; color: #94a3b8; line-height: 1.6; margin: 0; }}
</style>
</head>
<body>
  <div class="card">
    <div class="badge">{badge}</div>
    <h1>Local-AI-Route</h1>
    <p>{safe_message}</p>
  </div>
  <script>setTimeout(() => {{ try {{ window.close(); }} catch(e) {{}} }}, 2000);</script>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_code_and_state_variants() {
        let (c1, s1) = extract_code_and_state(
            "http://localhost:1455/auth/callback?code=auth_code_123&state=st_456",
        )
        .unwrap();
        assert_eq!(c1, "auth_code_123");
        assert_eq!(s1.as_deref(), Some("st_456"));

        let (c2, s2) = extract_code_and_state("claude_code_abc#claude_state_xyz").unwrap();
        assert_eq!(c2, "claude_code_abc");
        assert_eq!(s2.as_deref(), Some("claude_state_xyz"));

        let (c3, s3) = extract_code_and_state("raw_code_only").unwrap();
        assert_eq!(c3, "raw_code_only");
        assert!(s3.is_none());
    }

    #[test]
    fn test_escape_html_prevents_xss() {
        let html = build_callback_html(false, "<script>alert('xss')</script>&\"");
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;&amp;&quot;"));
    }
}
