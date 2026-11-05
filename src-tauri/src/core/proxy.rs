use super::models::{ProxyConfig, DEFAULT_NO_PROXY};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Duration;

pub const PROXY_ENV_KEYS: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    "NODE_USE_ENV_PROXY",
];

pub fn build_proxy_env_vars(proxy: &ProxyConfig) -> Vec<(String, String)> {
    let proxy_url = proxy.url();
    let no_proxy_env = format_no_proxy_env(&proxy.no_proxy);
    vec![
        ("HTTP_PROXY".into(), proxy_url.clone()),
        ("HTTPS_PROXY".into(), proxy_url.clone()),
        ("ALL_PROXY".into(), proxy_url.clone()),
        ("NO_PROXY".into(), no_proxy_env.clone()),
        ("http_proxy".into(), proxy_url.clone()),
        ("https_proxy".into(), proxy_url.clone()),
        ("all_proxy".into(), proxy_url),
        ("no_proxy".into(), no_proxy_env),
        ("NODE_USE_ENV_PROXY".into(), "1".into()),
    ]
}

pub fn split_bypass_entries(raw: &str) -> Vec<String> {
    if raw.trim().is_empty() {
        return vec![];
    }
    let normalized = raw.replace(';', ",").replace('\r', ",").replace('\n', ",");
    let mut seen = HashSet::new();
    let mut items = Vec::new();

    for part in normalized.split(',') {
        let token = part.trim();
        if token.is_empty() {
            continue;
        }
        let lower = token.to_lowercase();
        if !seen.contains(&lower) {
            seen.insert(lower);
            items.push(token.to_string());
        }
    }
    items
}

pub fn format_chromium_bypass_list(raw: &str) -> String {
    let mut entries = split_bypass_entries(raw);
    if entries.is_empty() {
        entries = split_bypass_entries(DEFAULT_NO_PROXY);
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut has_localhost = false;

    for item in entries {
        let mut token = item;
        if token == "::1" {
            token = "[::1]".to_string();
        }
        if token.to_lowercase() == "localhost" {
            has_localhost = true;
        }
        let key = token.to_lowercase();
        if !seen.contains(&key) {
            seen.insert(key);
            out.push(token);
        }
    }

    if has_localhost && !seen.contains("<local>") {
        out.push("<local>".to_string());
    }

    out.join(";")
}

pub fn format_no_proxy_env(raw: &str) -> String {
    let entries = split_bypass_entries(raw);
    if entries.is_empty() {
        DEFAULT_NO_PROXY.to_string()
    } else {
        entries.join(",")
    }
}

pub async fn scan_local_proxy_ports() -> Vec<u16> {
    let common_ports: &[u16] = &[
        7890, 7891, 10808, 10809, 1080, 1081, 1082, 8888, 8889, 7892, 7893, 7895, 20170, 20171,
    ];

    let mut join_set = tokio::task::JoinSet::new();
    for &port in common_ports {
        join_set.spawn(async move {
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            if let Ok(Ok(_)) = tokio::time::timeout(
                Duration::from_millis(60),
                tokio::net::TcpStream::connect(addr),
            )
            .await
            {
                Some(port)
            } else {
                None
            }
        });
    }

    let mut active = Vec::new();
    while let Some(res) = join_set.join_next().await {
        if let Ok(Some(port)) = res {
            active.push(port);
        }
    }
    active.sort_unstable();
    active
}
