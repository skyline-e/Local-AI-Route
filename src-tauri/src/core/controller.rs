use super::models::{AppRuntimeState, ProxyConfig, RunStatus, TargetAppSpec};
use super::proxy::{format_chromium_bypass_list, format_no_proxy_env};
use crate::platforms::request_graceful_close;
use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::process::Command;
use sysinfo::{Pid, ProcessesToUpdate, System};

pub fn is_chromium_helper(cmd: &[String]) -> bool {
    cmd.iter().any(|arg| arg.starts_with("--type=") || arg == "--crashpad-handler")
}

pub fn extract_proxy_arg(cmd: &[String]) -> Option<String> {
    for arg in cmd {
        if let Some(stripped) = arg.strip_prefix("--proxy-server=") {
            return Some(stripped.trim().trim_matches('"').to_string());
        }
    }
    None
}

pub fn is_executable_present(path: &Path) -> bool {
    path.is_file() || std::fs::symlink_metadata(path).map(|m| !m.is_dir()).unwrap_or(false)
}

fn is_matching_exe_name(actual_name: &str, target_name: &str) -> bool {
    actual_name == target_name
        || matches!(
            (actual_name, target_name),
            ("chatgpt.exe", "chatgpt-desktop.exe") | ("chatgpt-desktop.exe", "chatgpt.exe")
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

    let spec_path = Path::new(&spec.exe_path);
    let spec_path_norm = spec.exe_path.replace('/', "\\").to_lowercase();
    let target_file_name = spec_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Precompile regex outside process iteration loop for optimal performance
    let compiled_regex = if !spec.sidecar_cmdline_regex.is_empty() {
        regex::Regex::new(&spec.sidecar_cmdline_regex).ok()
    } else {
        None
    };

    let mut matched_pids = Vec::new();
    let mut main_cmdline = Vec::new();

    for (pid, process) in sys.processes() {
        let p_name = process.name().to_string_lossy().to_lowercase();
        let cmd: Vec<String> = process.cmd().iter().map(|s| s.to_string_lossy().to_string()).collect();
        let cmd_joined = cmd.join(" ");

        let exe_matches = if let Some(exe) = process.exe() {
            let exe_norm = exe.to_string_lossy().replace('/', "\\").to_lowercase();
            if exe_norm == spec_path_norm {
                if let Some(ref re) = compiled_regex {
                    re.is_match(&cmd_joined)
                } else {
                    true
                }
            } else if exe
                .file_name()
                .and_then(|f| f.to_str())
                .map(|s| is_matching_exe_name(&s.to_lowercase(), &target_file_name))
                .unwrap_or(false)
            {
                if let Some(ref re) = compiled_regex {
                    re.is_match(&cmd_joined)
                } else {
                    is_same_app_family(spec_path, &spec_path_norm, exe, &exe_norm)
                }
            } else {
                false
            }
        } else {
            let name_matches = is_matching_exe_name(&p_name, &target_file_name);
            if name_matches {
                if let Some(ref re) = compiled_regex {
                    re.is_match(&cmd_joined)
                } else if let Some(first_arg) = cmd.first() {
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

        if is_chromium_helper(&cmd) {
            continue;
        }

        matched_pids.push(pid.as_u32());
        if main_cmdline.is_empty() || (extract_proxy_arg(&main_cmdline).is_none() && extract_proxy_arg(&cmd).is_some()) {
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

    let detected_proxy = extract_proxy_arg(&main_cmdline);
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

    let mut cmd = Command::new(&spec.exe_path);
    let exe_low = spec.exe_path.replace('/', "\\").to_lowercase();
    if !exe_low.contains("\\microsoft\\windowsapps\\") {
        if let Some(parent) = Path::new(&spec.exe_path).parent() {
            if parent.is_dir() {
                cmd.current_dir(parent);
            }
        }
    }

    if spec.proxy_enabled {
        let proxy_url = proxy.url();
        let bypass_list = format_chromium_bypass_list(&proxy.no_proxy);
        let no_proxy_env = format_no_proxy_env(&proxy.no_proxy);

        cmd.arg(format!("--proxy-server={}", proxy_url));
        cmd.arg(format!("--proxy-bypass-list={}", bypass_list));
        cmd.arg("--disable-quic");

        cmd.env("HTTP_PROXY", &proxy_url);
        cmd.env("HTTPS_PROXY", &proxy_url);
        cmd.env("ALL_PROXY", &proxy_url);
        cmd.env("NO_PROXY", &no_proxy_env);
        cmd.env("http_proxy", &proxy_url);
        cmd.env("https_proxy", &proxy_url);
        cmd.env("all_proxy", &proxy_url);
        cmd.env("no_proxy", &no_proxy_env);
        cmd.env("NODE_USE_ENV_PROXY", "1");
    } else {
        cmd.env_remove("HTTP_PROXY");
        cmd.env_remove("HTTPS_PROXY");
        cmd.env_remove("ALL_PROXY");
        cmd.env_remove("NO_PROXY");
        cmd.env_remove("http_proxy");
        cmd.env_remove("https_proxy");
        cmd.env_remove("all_proxy");
        cmd.env_remove("no_proxy");
        cmd.env_remove("NODE_USE_ENV_PROXY");
    }

    let child = cmd.spawn().map_err(|e| format!("Failed to spawn process: {}", e))?;
    Ok(child.id())
}

pub fn stop_target(pids: &[u32], force: bool) -> Result<(), String> {
    if pids.is_empty() {
        return Ok(());
    }

    if !force {
        request_graceful_close(pids);
    } else {
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let tree = collect_process_tree(&sys, pids);
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
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let mut tracked_tree = collect_process_tree(&sys, pids);

    request_graceful_close(pids);

    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(200));
        sys.refresh_processes(ProcessesToUpdate::All, true);
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
    sys.refresh_processes(ProcessesToUpdate::All, true);
    tracked_tree.extend(collect_process_tree(&sys, pids));
    for pid in tracked_tree {
        if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            let _ = proc.kill();
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    true
}
