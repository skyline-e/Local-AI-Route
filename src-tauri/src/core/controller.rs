use super::models::{AppRuntimeState, ProxyConfig, RunStatus, TargetAppSpec};
use super::proxy::{build_proxy_env_vars, format_chromium_bypass_list, PROXY_ENV_KEYS};
use crate::platforms::request_graceful_close;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::Path;
use std::process::Command;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

pub fn refresh_processes_for_inspection(sys: &mut System, include_env: bool) {
    let mut kind = ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet);
    if include_env {
        kind = kind.with_environ(UpdateKind::OnlyIfNotSet);
    }
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
}

pub fn is_versioned_package_path(path: &str) -> bool {
    let norm = path.replace('/', "\\").to_lowercase();
    (norm.contains("\\windowsapps\\") && !norm.contains("\\microsoft\\windowsapps\\"))
        || norm.contains("\\app-")
}

pub fn is_cli_binary_for_app(app_id: &str, path: &str) -> bool {
    let fname = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    match app_id {
        "factory" => matches!(fname.as_str(), "droid.exe" | "droid" | "dsw.exe" | "dsw"),
        _ => false,
    }
}

pub fn should_heal_builtin_path(app_id: &str, exe_path: &str) -> bool {
    let p = Path::new(exe_path);
    exe_path.is_empty()
        || p.is_dir()
        || !is_executable_present(p)
        || is_versioned_package_path(exe_path)
        || is_cli_binary_for_app(app_id, exe_path)
}

pub fn is_chromium_helper(cmd: &[String]) -> bool {
    cmd.iter().any(|arg| {
        let low = arg.to_lowercase();
        low.starts_with("--type=")
            || low == "--crashpad-handler"
            || low == "--node-ipc"
            || low == "--usenodeipc"
            || low.starts_with("--clientprocessid=")
            || low == "--ms-enable-electron-run-as-node"
    })
}

pub fn extract_proxy_arg(cmd: &[String]) -> Option<String> {
    for arg in cmd {
        let cleaned = arg.trim().trim_matches('"');
        if let Some(stripped) = cleaned.strip_prefix("--proxy-server=") {
            return Some(stripped.trim().trim_matches('"').to_string());
        }
    }
    None
}

pub fn get_gemini_profile_proxy() -> Option<String> {
    if let Some(appdata) = dirs::config_dir() {
        for sub in &["gemini", "Gemini", "Google/Gemini", "google-gemini"] {
            let pref_path = appdata.join(sub).join("Preferences");
            if pref_path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&pref_path) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(proxy_obj) = val.get("proxy") {
                            if let Some(server) = proxy_obj.get("server").and_then(|s| s.as_str()) {
                                return Some(server.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

pub fn sync_gemini_profile_proxy(proxy: &ProxyConfig, enabled: bool) {
    if let Some(appdata) = dirs::config_dir() {
        let mut target_dir = None;
        for sub in &["gemini", "Gemini", "Google/Gemini", "google-gemini"] {
            let dir = appdata.join(sub);
            if dir.is_dir() {
                target_dir = Some(dir);
                break;
            }
        }
        let target_dir = target_dir.unwrap_or_else(|| appdata.join("gemini"));
        let pref_path = target_dir.join("Preferences");
        let mut val: serde_json::Value = if pref_path.is_file() {
            match std::fs::read_to_string(&pref_path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                    eprintln!("[WARN] 解析 Gemini Preferences 失败: {}", e);
                    serde_json::json!({})
                }),
                Err(_) => serde_json::json!({}),
            }
        } else {
            serde_json::json!({})
        };

        if enabled {
            val["proxy"] = serde_json::json!({
                "mode": "fixed_servers",
                "server": format!("{}:{}", proxy.host, proxy.port),
                "bypass_list": format_chromium_bypass_list(&proxy.no_proxy)
            });
        } else if let Some(obj) = val.as_object_mut() {
            obj.remove("proxy");
        }

        let _ = std::fs::create_dir_all(&target_dir);
        if let Ok(new_content) = serde_json::to_string_pretty(&val) {
            // Write atomically via a temporary file to prevent corruption
            let tmp_path = target_dir.join("Preferences.tmp");
            if std::fs::write(&tmp_path, new_content).is_ok() {
                let _ = std::fs::rename(&tmp_path, &pref_path);
            }
        }
    }
}

pub fn is_executable_present(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        if path.is_dir()
            && (path.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("app"))
                || path.join("Contents/MacOS").is_dir())
        {
            let macos_dir = path.join("Contents").join("MacOS");
            if macos_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(macos_dir) {
                    return entries.filter_map(|e| e.ok()).any(|e| e.path().is_file());
                }
            }
        }
    }

    if !path.is_dir() {
        if path.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = path.metadata() {
                    if meta.permissions().mode() & 0o111 == 0 {
                        return false;
                    }
                }
            }
            return true;
        }

        if std::fs::symlink_metadata(path).map(|m| !m.is_dir()).unwrap_or(false) {
            return true;
        }
    }

    #[cfg(target_os = "windows")]
    {
        let resolved = crate::platforms::resolve_executable_for_launch(&path.to_string_lossy());
        let res_path = Path::new(&resolved);
        if !res_path.is_dir()
            && (res_path.is_file()
                || std::fs::symlink_metadata(res_path)
                    .map(|m| !m.is_dir())
                    .unwrap_or(false))
        {
            return true;
        }
    }

    false
}

fn is_matching_exe_name(actual_name: &str, target_name: &str) -> bool {
    let clean_actual = actual_name.strip_suffix(".exe").unwrap_or(actual_name);
    let clean_target = target_name.strip_suffix(".exe").unwrap_or(target_name);
    clean_actual == clean_target
        || clean_actual.replace('-', " ") == clean_target.replace('-', " ")
        || matches!(
            (clean_actual, clean_target),
            ("chatgpt", "chatgpt-desktop")
                | ("chatgpt-desktop", "chatgpt")
                | ("claude", "claude-desktop")
                | ("claude-desktop", "claude")
                | ("gemini", "gemini-desktop")
                | ("gemini-desktop", "gemini")
                | ("factory", "factory-desktop")
                | ("factory-desktop", "factory")
        )
}

fn resolve_squirrel_root(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    let parent_name = parent.file_name()?.to_str()?.to_lowercase();
    let root = if parent_name.starts_with("app-") {
        parent.parent()?
    } else {
        parent
    };
    Some(format!("{}\\", root.to_string_lossy().replace('/', "\\").to_lowercase()))
}

fn is_same_app_family(spec_path: &Path, spec_path_norm: &str, exe_path: &Path, exe_norm: &str) -> bool {
    // 0. Canonical path equivalence (e.g. symlinks in /usr/bin -> /opt on Linux)
    if let (Ok(p1), Ok(p2)) = (std::fs::canonicalize(spec_path), std::fs::canonicalize(exe_path)) {
        if p1 == p2 {
            return true;
        }
    }

    // 1. MSIX Execution Alias (%LOCALAPPDATA%\Microsoft\WindowsApps\*.exe) <-> PackageRootFolder (C:\Program Files\WindowsApps\...)
    if spec_path_norm.contains("\\windowsapps\\") && exe_norm.contains("\\windowsapps\\") {
        return true;
    }

    // 2. Squirrel install root stub (Root\App.exe) <-> versioned subdir (Root\app-x.y.z\App.exe)
    let spec_has_app_dir = spec_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_lowercase().starts_with("app-"))
        .unwrap_or(false);
    let exe_has_app_dir = exe_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_lowercase().starts_with("app-"))
        .unwrap_or(false);

    if spec_has_app_dir || exe_has_app_dir {
        if let (Some(spec_root), Some(exe_root)) = (resolve_squirrel_root(spec_path), resolve_squirrel_root(exe_path)) {
            if spec_root == exe_root {
                return true;
            }
        }
    }

    false
}

pub fn inspect_app_state(sys: &System, spec: &TargetAppSpec, proxy: &ProxyConfig) -> AppRuntimeState {
    if spec.exe_path.is_empty() {
        return AppRuntimeState {
            app_id: spec.id.clone(),
            status: RunStatus::Stopped,
            pids: vec![],
            exe_path: String::new(),
            matched_proxy: String::new(),
            cmdline_preview: String::new(),
        };
    }

    let effective_exe_str = crate::platforms::resolve_executable_for_launch(&spec.exe_path);
    let spec_path = Path::new(&effective_exe_str);
    let spec_path_norm = effective_exe_str.replace('/', "\\").to_lowercase();
    let target_file_name = spec_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_lowercase();
    let raw_target_file_name = Path::new(&spec.exe_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mut all_candidate_pids = HashSet::new();
    let mut helper_pids = HashSet::new();
    let mut non_helper_candidates: BTreeMap<u32, Vec<String>> = BTreeMap::new();

    for (pid, process) in sys.processes() {
        let p_name = process.name().to_string_lossy().to_lowercase();
        let cmd: Vec<String> = process.cmd().iter().map(|s| s.to_string_lossy().to_string()).collect();

        let exe_matches = if let Some(exe) = process.exe() {
            let exe_norm = exe.to_string_lossy().replace('/', "\\").to_lowercase();
            if exe_norm == spec_path_norm {
                true
            } else if exe
                .file_name()
                .and_then(|f| f.to_str())
                .map(|s| {
                    let s_low = s.to_lowercase();
                    is_matching_exe_name(&s_low, &target_file_name)
                        || (!raw_target_file_name.is_empty()
                            && is_matching_exe_name(&s_low, &raw_target_file_name))
                })
                .unwrap_or(false)
            {
                is_same_app_family(spec_path, &spec_path_norm, exe, &exe_norm)
            } else {
                false
            }
        } else {
            let name_matches = is_matching_exe_name(&p_name, &target_file_name)
                || (!raw_target_file_name.is_empty()
                    && is_matching_exe_name(&p_name, &raw_target_file_name));
            if name_matches {
                if let Some(first_arg) = cmd.first() {
                    let cleaned_arg = first_arg.trim().trim_matches('"');
                    let cmd_exe = Path::new(cleaned_arg);
                    let cmd_exe_norm = cleaned_arg.replace('/', "\\").to_lowercase();
                    cmd_exe_norm == spec_path_norm
                        || is_same_app_family(spec_path, &spec_path_norm, cmd_exe, &cmd_exe_norm)
                } else {
                    false
                }
            } else {
                false
            }
        };

        if !exe_matches {
            continue;
        }

        let pid_u32 = pid.as_u32();
        all_candidate_pids.insert(pid_u32);

        if is_chromium_helper(&cmd) {
            helper_pids.insert(pid_u32);
            continue;
        }

        non_helper_candidates.insert(pid_u32, cmd);
    }

    // Filter out any candidate whose ancestor is a Chromium/Electron helper process (e.g. Node extension workers)
    let mut matched_pids = Vec::new();
    let mut main_cmdline = Vec::new();
    let mut best_rank: Option<(bool, bool, bool)> = None;

    for (pid_u32, cmd) in non_helper_candidates {
        let mut curr = sys.process(Pid::from_u32(pid_u32)).and_then(|p| p.parent());
        let mut has_helper_ancestor = false;
        let mut has_candidate_ancestor = false;
        let mut visited = HashSet::new();

        while let Some(parent_pid) = curr {
            let pp_u32 = parent_pid.as_u32();
            if !visited.insert(pp_u32) {
                break;
            }
            if helper_pids.contains(&pp_u32) {
                has_helper_ancestor = true;
                break;
            }
            if all_candidate_pids.contains(&pp_u32) {
                has_candidate_ancestor = true;
            }
            curr = sys.process(parent_pid).and_then(|p| p.parent());
        }

        if has_helper_ancestor {
            continue;
        }

        let is_root = !has_candidate_ancestor;
        matched_pids.push(pid_u32);

        let cmd_has_proxy = extract_proxy_arg(&cmd).is_some();
        let has_non_empty_cmd = cmd.iter().any(|s| !s.is_empty());
        let rank = (cmd_has_proxy, has_non_empty_cmd, is_root);

        if best_rank.map_or(true, |prev| rank > prev) {
            best_rank = Some(rank);
            main_cmdline = cmd;
        }
    }

    if matched_pids.is_empty() {
        return AppRuntimeState {
            app_id: spec.id.clone(),
            status: RunStatus::Stopped,
            pids: vec![],
            exe_path: spec.exe_path.clone(),
            matched_proxy: String::new(),
            cmdline_preview: String::new(),
        };
    }

    let detected_proxy = if spec.id == "gemini" {
        get_gemini_profile_proxy()
    } else {
        extract_proxy_arg(&main_cmdline)
    };
    let expected_authority = proxy.authority().to_lowercase();

    let status = if !spec.proxy_enabled {
        if detected_proxy.is_none() {
            RunStatus::RunningDirect
        } else {
            RunStatus::RestartNeeded
        }
    } else {
        match detected_proxy {
            Some(ref detected) => {
                let det_low = detected.to_lowercase();
                if det_low.contains(&expected_authority) {
                    RunStatus::RunningProxied
                } else {
                    RunStatus::RestartNeeded
                }
            }
            None => RunStatus::RunningUnproxied,
        }
    };

    AppRuntimeState {
        app_id: spec.id.clone(),
        status,
        pids: matched_pids,
        exe_path: spec.exe_path.clone(),
        matched_proxy: detected_proxy.unwrap_or_default(),
        cmdline_preview: main_cmdline.join(" "),
    }
}

pub fn collect_process_tree(sys: &System, root_pids: &[u32]) -> HashSet<u32> {
    let mut all = HashSet::new();
    let mut queue = VecDeque::new();

    for &pid in root_pids {
        all.insert(pid);
        queue.push_back(Pid::from_u32(pid));
    }

    while let Some(curr_pid) = queue.pop_front() {
        for (pid, proc) in sys.processes() {
            if proc.parent() == Some(curr_pid) {
                let p_u32 = pid.as_u32();
                if all.insert(p_u32) {
                    queue.push_back(*pid);
                }
            }
        }
    }
    all
}

pub fn launch_target(spec: &TargetAppSpec, proxy: &ProxyConfig) -> Result<u32, String> {
    if spec.exe_path.is_empty() || !is_executable_present(Path::new(&spec.exe_path)) {
        return Err(format!("Target executable does not exist: {}", spec.exe_path));
    }

    let resolved_exe = crate::platforms::resolve_executable_for_launch(&spec.exe_path);
    let mut cmd = Command::new(&resolved_exe);
    let exe_low = resolved_exe.replace('/', "\\").to_lowercase();
    if !exe_low.contains("\\windowsapps\\") {
        if let Some(parent) = Path::new(&resolved_exe).parent() {
            if parent.is_dir() {
                cmd.current_dir(parent);
            }
        }
    }

    if spec.id == "gemini" {
        sync_gemini_profile_proxy(proxy, spec.proxy_enabled);
    }

    let mut cli_args: Vec<String> = Vec::new();
    let mut env_vars: Vec<(String, String)> = Vec::new();
    if spec.proxy_enabled {
        let proxy_url = proxy.url();
        let bypass_list = format_chromium_bypass_list(&proxy.no_proxy);

        if spec.id != "gemini" {
            let proxy_arg = format!("--proxy-server={}", proxy_url);
            let bypass_arg = format!("--proxy-bypass-list={}", bypass_list);
            cmd.arg(&proxy_arg);
            cmd.arg(&bypass_arg);
            cli_args.push(proxy_arg);
            cli_args.push(bypass_arg);
        }
        cmd.arg("--disable-quic");
        cli_args.push("--disable-quic".to_string());

        env_vars = build_proxy_env_vars(proxy);
        for (k, v) in &env_vars {
            cmd.env(k, v);
        }
    } else {
        for &k in PROXY_ENV_KEYS {
            cmd.env_remove(k);
        }
    }

    match cmd.spawn() {
        Ok(child) => Ok(child.id()),
        Err(e) => {
            #[cfg(target_os = "windows")]
            {
                let raw_err = e.raw_os_error();
                let is_access_or_elevation = raw_err == Some(5)
                    || raw_err == Some(740)
                    || e.kind() == std::io::ErrorKind::PermissionDenied;
                if is_access_or_elevation {
                    // 1. Try MSIX ApplicationActivationManager COM activation (for Store / WindowsApps packages)
                    if let Ok(pid) = crate::platforms::windows::activate_msix_application(
                        &spec.id,
                        &resolved_exe,
                        &cli_args,
                    )
                    .or_else(|_| {
                        crate::platforms::windows::activate_msix_application(
                            &spec.id,
                            &spec.exe_path,
                            &cli_args,
                        )
                    }) {
                        return Ok(pid);
                    }

                    // 2. Fallback to ShellExecuteExW for apps requiring UAC elevation (os error 740 / RunAsAdmin)
                    // or shell-mediated execution (os error 5 on reparse points / protected launchers)
                    let work_dir = if !exe_low.contains("\\windowsapps\\") {
                        Path::new(&resolved_exe)
                            .parent()
                            .filter(|p| p.is_dir())
                            .map(|p| p.to_string_lossy().to_string())
                    } else {
                        None
                    };
                    if let Ok(pid) = crate::platforms::windows::shell_execute_application(
                        &resolved_exe,
                        &cli_args,
                        work_dir.as_deref(),
                        &env_vars,
                        spec.proxy_enabled,
                    ) {
                        return Ok(pid);
                    }
                }
            }
            Err(format!("Failed to spawn process: {}", e))
        }
    }
}

pub fn stop_target(sys: &System, pids: &[u32], force: bool) -> Result<(), String> {
    if pids.is_empty() {
        return Ok(());
    }

    if !force {
        request_graceful_close(pids);
    } else {
        let tree = collect_process_tree(sys, pids);
        for pid in tree {
            if let Some(proc) = sys.process(Pid::from_u32(pid)) {
                let _ = proc.kill();
            }
        }
    }
    Ok(())
}

pub fn ensure_stopped_before_relaunch(pids: &[u32], allow_force_kill: bool) -> bool {
    if pids.is_empty() {
        return true;
    }

    let mut sys = System::new();
    refresh_processes_for_inspection(&mut sys, false);
    let mut tracked_tree = collect_process_tree(&sys, pids);

    request_graceful_close(pids);

    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(200));
        refresh_processes_for_inspection(&mut sys, false);
        let current_tree = collect_process_tree(&sys, pids);
        tracked_tree.extend(current_tree);

        let any_root_alive = pids.iter().any(|&p| sys.process(Pid::from_u32(p)).is_some());
        if !any_root_alive {
            // Clean up any leftover orphaned helper processes from the pre-close tree snapshot
            for &pid in &tracked_tree {
                if let Some(proc) = sys.process(Pid::from_u32(pid)) {
                    let _ = proc.kill();
                }
            }
            return true;
        }
    }

    if !allow_force_kill {
        return false;
    }

    // If app minimized to tray on WM_CLOSE or user explicitly confirmed force restart, terminate tracked tree
    refresh_processes_for_inspection(&mut sys, false);
    tracked_tree.extend(collect_process_tree(&sys, pids));
    for pid in tracked_tree {
        if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            let _ = proc.kill();
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_versioned_package_path() {
        assert!(is_versioned_package_path(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.1027.3384.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
        ));
        assert!(is_versioned_package_path(
            r"C:\Users\apuser\AppData\Local\AnthropicClaude\app-0.7.8\Claude.exe"
        ));
        assert!(!is_versioned_package_path(
            r"C:\Users\apuser\AppData\Local\Microsoft\WindowsApps\chatgpt-desktop.exe"
        ));
    }

    #[test]
    fn test_is_cli_binary_for_app() {
        assert!(is_cli_binary_for_app("factory", r"C:\Users\apuser\bin\droid.exe"));
        assert!(is_cli_binary_for_app("factory", r"/usr/local/bin/droid"));
        assert!(!is_cli_binary_for_app("custom-app", r"C:\Users\apuser\bin\droid.exe"));
        assert!(!is_cli_binary_for_app(
            "factory",
            r"C:\Users\apuser\AppData\Local\Factory\app-1.6.0\factory-desktop.exe"
        ));
    }
}
