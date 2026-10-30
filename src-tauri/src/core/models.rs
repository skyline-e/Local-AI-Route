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

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: "zh-CN".into(),
            theme: "system".into(),
            autostart: false,
            proxy: ProxyConfig::default(),
            targets: HashMap::new(),
            custom_apps: Vec::new(),
        }
    }
}
