use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DEFAULT_NO_PROXY: &str =
    "localhost,127.0.0.1,::1,[::1],*.local,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Stopped,
    RunningProxied,
    RunningDirect,
    RunningUnproxied,
    RestartNeeded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub no_proxy: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            scheme: "http".into(),
            host: "127.0.0.1".into(),
            port: 7890,
            no_proxy: DEFAULT_NO_PROXY.into(),
        }
    }
}

impl ProxyConfig {
    pub fn authority(&self) -> String {
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub fn url(&self) -> String {
        format!("{}://{}", self.scheme, self.authority())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetOverride {
    pub exe_path: String,
    pub proxy_enabled: bool,
}

impl Default for TargetOverride {
    fn default() -> Self {
        Self {
            exe_path: String::new(),
            proxy_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetAppSpec {
    pub id: String,
    pub name: String,
    pub name_en: String,
    pub builtin: bool,
    pub exe_path: String,
    pub proxy_enabled: bool,
    pub probe_urls: Vec<String>,
    pub child_process_patterns: Vec<String>,
    pub sidecar_cmdline_regex: String,
    pub warn_unsaved_on_kill: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRuntimeState {
    pub app_id: String,
    pub status: RunStatus,
    pub pids: Vec<u32>,
    pub exe_path: String,
    pub matched_proxy: String,
    pub cmdline_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagItem {
    pub title: String,
    pub status: CheckStatus,
    pub summary: String,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagReport {
    pub app_id: String,
    pub app_name: String,
    pub proxy_url: String,
    pub timestamp: String,
    pub items: Vec<DiagItem>,
    pub raw_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub language: String,
    pub theme: String,
    pub autostart: bool,
    pub proxy: ProxyConfig,
    pub targets: HashMap<String, TargetOverride>,
    pub custom_apps: Vec<TargetAppSpec>,
}

pub fn detect_default_language() -> String {
    // 1. Check POSIX / standard environment variables first (LC_ALL, LC_MESSAGES, LANG)
    let env_lang = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_lowercase();

    if !env_lang.is_empty() {
        if env_lang.starts_with("zh") {
            return "zh-CN".into();
        } else {
            return "en-US".into();
        }
    }

    // 2. On Windows, detect user default UI language via Win32 API
    #[cfg(target_os = "windows")]
    {
        let lang_id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
        // Primary language ID 0x04 is LANG_CHINESE (zh-CN 0x0804, zh-TW 0x0404, etc.)
        if (lang_id & 0x03ff) == 0x04 {
            return "zh-CN".into();
        } else {
            return "en-US".into();
        }
    }

    // Fallback to en-US for international environments / AppImage catalog compliance
    #[cfg(not(target_os = "windows"))]
    {
        "en-US".into()
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: detect_default_language(),
            theme: "system".into(),
            autostart: false,
            proxy: ProxyConfig::default(),
            targets: HashMap::new(),
            custom_apps: Vec::new(),
        }
    }
}
