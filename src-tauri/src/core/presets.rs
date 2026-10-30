use super::models::{AppSettings, TargetAppSpec};

pub fn builtin_presets() -> Vec<TargetAppSpec> {
    vec![
        TargetAppSpec {
            id: "chatgpt".into(),
            name: "ChatGPT APP".into(),
            name_en: "ChatGPT APP".into(),
            builtin: true,
            exe_path: String::new(),
            proxy_enabled: true,
            probe_urls: vec!["https://chatgpt.com/".into()],
            child_process_patterns: vec![],
            sidecar_cmdline_regex: String::new(),
            warn_unsaved_on_kill: false,
        },
        TargetAppSpec {
            id: "antigravity-app".into(),
            name: "Antigravity (桌面版)".into(),
            name_en: "Antigravity (Desktop)".into(),
            builtin: true,
            exe_path: String::new(),
            proxy_enabled: true,
            probe_urls: vec![
                "https://www.google.com/generate_204".into(),
                "https://generativelanguage.googleapis.com/".into(),
            ],
            child_process_patterns: vec![
                "language_server.exe".into(),
                "language_server_windows_x64.exe".into(),
                "language_server_windows_arm.exe".into(),
                "language_server_macos_arm".into(),
                "language_server_macos_x64".into(),
                "language_server".into(),
            ],
            sidecar_cmdline_regex: r"(?:^|\s)--standalone(?:\s|=|$)".into(),
            warn_unsaved_on_kill: false,
        },
        TargetAppSpec {
            id: "antigravity-ide".into(),
            name: "Antigravity IDE".into(),
            name_en: "Antigravity IDE".into(),
            builtin: true,
            exe_path: String::new(),
            proxy_enabled: true,
            probe_urls: vec![
                "https://www.google.com/generate_204".into(),
                "https://generativelanguage.googleapis.com/".into(),
            ],
            child_process_patterns: vec![
                "language_server_windows_x64.exe".into(),
                "language_server_windows_arm.exe".into(),
                "language_server_macos_arm".into(),
                "language_server_macos_x64".into(),
                "language_server".into(),
            ],
            sidecar_cmdline_regex: r#"--subclient_type(?:=|\s+)"?ide(?:"|\s|$)"#.into(),
            warn_unsaved_on_kill: true,
        },
    ]
}

pub fn get_effective_targets(settings: &AppSettings) -> Vec<TargetAppSpec> {
    let mut targets = builtin_presets();
    for target in &mut targets {
        if let Some(ov) = settings.targets.get(&target.id) {
            target.exe_path = ov.exe_path.clone();
            target.proxy_enabled = ov.proxy_enabled;
        }
    }
    for custom in &settings.custom_apps {
        targets.push(custom.clone());
    }
    targets
}

pub fn find_target_spec(settings: &AppSettings, app_id: &str) -> Option<TargetAppSpec> {
    get_effective_targets(settings).into_iter().find(|t| t.id == app_id)
}
