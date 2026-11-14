use super::models::{AppRuntimeState, ProxyConfig, RunStatus, TargetAppSpec};
use super::proxy::{build_proxy_env_vars, format_chromium_bypass_list, PROXY_ENV_KEYS};
use crate::platforms::{force_kill_pid, is_protected_pid, request_graceful_close};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::process::Command;
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

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

fn is_process_alive(proc: &sysinfo::Process) -> bool {
    !matches!(proc.status(), ProcessStatus::Zombie)
}

struct SpecMatchContext {
    effective_exe: String,
    spec_path_norm: String,
    target_file_name: String,
    raw_target_file_name: String,
}

impl SpecMatchContext {
    fn from_spec(spec: &TargetAppSpec) -> Option<Self> {
        if spec.exe_path.is_empty() {
            return None;
        }
        let effective_exe = crate::platforms::resolve_executable_for_launch(&spec.exe_path);
        let spec_path = Path::new(&effective_exe);
        let spec_path_norm = effective_exe.replace('/', "\\").to_lowercase();
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
        Some(Self {
            effective_exe,
            spec_path_norm,
            target_file_name,
            raw_target_file_name,
        })
    }

    fn matches_process(&self, process: &sysinfo::Process, cmd: &[String]) -> bool {
        let spec_path = Path::new(&self.effective_exe);
        let p_name = process.name().to_string_lossy().to_lowercase();

        if let Some(exe) = process.exe() {
            let exe_norm = exe.to_string_lossy().replace('/', "\\").to_lowercase();
            if exe_norm == self.spec_path_norm {
                return true;
            }
            if exe
                .file_name()
                .and_then(|f| f.to_str())
                .map(|s| {
                    let s_low = s.to_lowercase();
                    is_matching_exe_name(&s_low, &self.target_file_name)
                        || (!self.raw_target_file_name.is_empty()
                            && is_matching_exe_name(&s_low, &self.raw_target_file_name))
                })
                .unwrap_or(false)
            {
                return is_same_app_family(spec_path, &self.spec_path_norm, exe, &exe_norm);
            }
            return false;
        }

        let name_matches = is_matching_exe_name(&p_name, &self.target_file_name)
            || (!self.raw_target_file_name.is_empty()
                && is_matching_exe_name(&p_name, &self.raw_target_file_name));
        if name_matches {
            if let Some(first_arg) = cmd.first() {
                let cleaned_arg = first_arg.trim().trim_matches('"');
                let cmd_exe = Path::new(cleaned_arg);
                let cmd_exe_norm = cleaned_arg.replace('/', "\\").to_lowercase();
                return cmd_exe_norm == self.spec_path_norm
                    || is_same_app_family(spec_path, &self.spec_path_norm, cmd_exe, &cmd_exe_norm);
            }
        }
        false
    }
}

pub fn inspect_app_state(sys: &System, spec: &TargetAppSpec, proxy: &ProxyConfig) -> AppRuntimeState {
    let Some(ctx) = SpecMatchContext::from_spec(spec) else {
        return AppRuntimeState {
            app_id: spec.id.clone(),
            status: RunStatus::Stopped,
            pids: vec![],
            exe_path: String::new(),
            matched_proxy: String::new(),
            cmdline_preview: String::new(),
        };
    };

    let mut all_candidate_pids = HashSet::new();
    let mut helper_pids = HashSet::new();
    let mut non_helper_candidates: BTreeMap<u32, Vec<String>> = BTreeMap::new();

    for (pid, process) in sys.processes() {
        if !is_process_alive(process) {
            continue;
        }
        let cmd: Vec<String> = process.cmd().iter().map(|s| s.to_string_lossy().to_string()).collect();

        if !ctx.matches_process(process, &cmd) {
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

    let mut cli_args = crate::platforms::resolve_launch_extra_args(&spec.exe_path, &resolved_exe);
    if !cli_args.is_empty() {
        cmd.args(&cli_args);
    }

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

/// Fingerprint used to guard against OS PID reuse across force-stop retries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub start_time: u64,
    pub name_low: String,
}

impl ProcessIdentity {
    fn from_process(proc: &sysinfo::Process) -> Self {
        Self {
            start_time: proc.start_time(),
            name_low: proc.name().to_string_lossy().to_lowercase(),
        }
    }

    fn matches(&self, proc: &sysinfo::Process) -> bool {
        is_process_alive(proc)
            && proc.start_time() == self.start_time
            && proc.name().to_string_lossy().to_lowercase() == self.name_low
    }
}

fn snapshot_single_identities(sys: &System, pids: &[u32]) -> HashMap<u32, ProcessIdentity> {
    let mut map = HashMap::new();
    for &pid in pids {
        if is_protected_pid(pid) {
            continue;
        }
        if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            if is_process_alive(proc) {
                map.insert(pid, ProcessIdentity::from_process(proc));
            }
        }
    }
    map
}

pub fn snapshot_process_identities(sys: &System, root_pids: &[u32]) -> HashMap<u32, ProcessIdentity> {
    if root_pids.is_empty() {
        return HashMap::new();
    }
    let tree: Vec<u32> = collect_process_tree(sys, root_pids).into_iter().collect();
    snapshot_single_identities(sys, &tree)
}

fn retain_live_identities(
    sys: &System,
    tracked: HashMap<u32, ProcessIdentity>,
) -> HashMap<u32, ProcessIdentity> {
    tracked
        .into_iter()
        .filter(|(pid, identity)| {
            !is_protected_pid(*pid)
                && sys
                    .process(Pid::from_u32(*pid))
                    .is_some_and(|proc| identity.matches(proc))
        })
        .collect()
}

fn matches_sidecar_process(
    process_name: &str,
    cmd_joined: &str,
    patterns_low: &[String],
    sidecar_regex: &regex::Regex,
) -> bool {
    let name_low = process_name.to_lowercase();
    patterns_low.iter().any(|pat| name_low.contains(pat)) && sidecar_regex.is_match(cmd_joined)
}

pub fn collect_target_identities_for_force_kill(
    sys: &System,
    spec: &TargetAppSpec,
) -> HashMap<u32, ProcessIdentity> {
    let Some(ctx) = SpecMatchContext::from_spec(spec) else {
        return HashMap::new();
    };

    let sidecar_regex =
        if !spec.sidecar_cmdline_regex.is_empty() && !spec.child_process_patterns.is_empty() {
            regex::Regex::new(&spec.sidecar_cmdline_regex).ok()
        } else {
            None
        };
    let patterns_low: Vec<String> = if sidecar_regex.is_some() {
        spec.child_process_patterns
            .iter()
            .map(|p| p.to_lowercase())
            .collect()
    } else {
        Vec::new()
    };

    let mut seeds = Vec::new();
    for (pid, process) in sys.processes() {
        let pid_u32 = pid.as_u32();
        if is_protected_pid(pid_u32) || !is_process_alive(process) {
            continue;
        }

        let cmd: Vec<String> = process
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        if ctx.matches_process(process, &cmd) {
            seeds.push(pid_u32);
            continue;
        }

        if let Some(ref re) = sidecar_regex {
            let name = process.name().to_string_lossy();
            let cmd_joined = cmd.join(" ");
            if matches_sidecar_process(&name, &cmd_joined, &patterns_low, re) {
                seeds.push(pid_u32);
            }
        }
    }

    snapshot_process_identities(sys, &seeds)
}

fn kill_pid_set(sys: &System, tracked: &HashMap<u32, ProcessIdentity>) {
    for &pid in tracked.keys() {
        if is_protected_pid(pid) {
            continue;
        }
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            let _ = force_kill_pid(pid);
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            if is_process_alive(proc) {
                let _ = proc.kill();
            }
        }
    }
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    let _ = sys;
}

/// Sends a graceful close signal (WM_CLOSE on Windows, SIGTERM on Unix) to the given PIDs.
pub fn graceful_stop_pids(pids: &[u32]) {
    if !pids.is_empty() {
        request_graceful_close(pids);
    }
}

/// Force-terminates the target app family, tracked child tree, and detached sidecars, verifying exit.
pub fn force_stop_target(
    spec: &TargetAppSpec,
    seed_identities: &HashMap<u32, ProcessIdentity>,
    is_en: bool,
) -> Result<(), String> {
    let mut sys = System::new();
    refresh_processes_for_inspection(&mut sys, false);

    let mut tracked = collect_target_identities_for_force_kill(&sys, spec);
    if !seed_identities.is_empty() {
        let live_seeds = retain_live_identities(&sys, seed_identities.clone());
        let seed_pids: Vec<u32> = live_seeds.keys().copied().collect();
        tracked.extend(live_seeds);
        tracked.extend(snapshot_process_identities(&sys, &seed_pids));
    }

    if tracked.is_empty() {
        return Ok(());
    }

    for _ in 0..12 {
        kill_pid_set(&sys, &tracked);
        std::thread::sleep(std::time::Duration::from_millis(80));
        refresh_processes_for_inspection(&mut sys, false);

        tracked = retain_live_identities(&sys, tracked);
        let live_pids: Vec<u32> = tracked.keys().copied().collect();
        if !live_pids.is_empty() {
            tracked.extend(snapshot_process_identities(&sys, &live_pids));
        }
        tracked.extend(collect_target_identities_for_force_kill(&sys, spec));

        if tracked.is_empty() {
            return Ok(());
        }
    }

    let mut remaining: Vec<String> = tracked
        .keys()
        .filter_map(|&pid| {
            sys.process(Pid::from_u32(pid))
                .filter(|p| is_process_alive(p))
                .map(|p| format!("{} (PID {})", p.name().to_string_lossy(), pid))
        })
        .collect();
    remaining.sort();
    remaining.truncate(5);

    let summary = remaining.join(", ");
    if is_en {
        Err(format!(
            "Failed to terminate remaining process(es) (may require elevated privileges): {}",
            summary
        ))
    } else {
        Err(format!(
            "以下残留进程未能强制结束（可能权限不足或受系统保护）: {}",
            summary
        ))
    }
}

pub fn ensure_stopped_before_relaunch(pids: &[u32], allow_force_kill: bool) -> bool {
    if pids.is_empty() {
        return true;
    }

    let mut sys = System::new();
    refresh_processes_for_inspection(&mut sys, false);
    let root_identities = snapshot_single_identities(&sys, pids);
    if root_identities.is_empty() {
        return true;
    }
    let mut tracked_tree = snapshot_process_identities(&sys, pids);

    request_graceful_close(pids);

    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(200));
        refresh_processes_for_inspection(&mut sys, false);
        tracked_tree = retain_live_identities(&sys, tracked_tree);

        let live_roots: Vec<u32> = retain_live_identities(&sys, root_identities.clone())
            .into_keys()
            .collect();
        if live_roots.is_empty() {
            // Clean up any leftover orphaned helper processes from the verified pre-close tree
            if !tracked_tree.is_empty() {
                kill_pid_set(&sys, &tracked_tree);
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
            return true;
        }
        tracked_tree.extend(snapshot_process_identities(&sys, &live_roots));
    }

    if !allow_force_kill {
        return false;
    }

    // If app minimized to tray on WM_CLOSE or user explicitly confirmed force restart, terminate tracked tree
    for _ in 0..8 {
        tracked_tree = retain_live_identities(&sys, tracked_tree);
        if tracked_tree.is_empty() {
            return true;
        }
        let live_pids: Vec<u32> = tracked_tree.keys().copied().collect();
        tracked_tree.extend(snapshot_process_identities(&sys, &live_pids));

        kill_pid_set(&sys, &tracked_tree);
        std::thread::sleep(std::time::Duration::from_millis(80));
        refresh_processes_for_inspection(&mut sys, false);
    }

    retain_live_identities(&sys, tracked_tree).is_empty()
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

    #[test]
    fn test_spec_match_context_and_sidecar_matching() {
        let mut spec = crate::core::presets::builtin_presets()
            .into_iter()
            .find(|s| s.id == "antigravity-app")
            .expect("antigravity-app preset");
        assert!(SpecMatchContext::from_spec(&spec).is_none());

        spec.exe_path = r"C:\Apps\Antigravity\Antigravity.exe".into();
        let ctx = SpecMatchContext::from_spec(&spec).expect("non-empty exe_path");
        assert_eq!(ctx.target_file_name, "antigravity.exe");

        let patterns_low: Vec<String> = spec
            .child_process_patterns
            .iter()
            .map(|p| p.to_lowercase())
            .collect();
        let re = regex::Regex::new(&spec.sidecar_cmdline_regex).expect("valid regex");
        assert!(matches_sidecar_process(
            "language_server_windows_x64.exe",
            "language_server_windows_x64.exe --standalone --port 8080",
            &patterns_low,
            &re
        ));
        assert!(!matches_sidecar_process(
            "language_server_windows_x64.exe",
            "language_server_windows_x64.exe --subclient_type=ide",
            &patterns_low,
            &re
        ));
        assert!(!matches_sidecar_process(
            "other_binary.exe",
            "other_binary.exe --standalone",
            &patterns_low,
            &re
        ));
    }

    fn spawn_test_child() -> std::process::Child {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            Command::new("ping")
                .args(["-n", "30", "127.0.0.1"])
                .creation_flags(0x08000000)
                .spawn()
                .expect("spawn ping")
        }
        #[cfg(not(target_os = "windows"))]
        {
            Command::new("sleep")
                .arg("30")
                .spawn()
                .expect("spawn sleep")
        }
    }

    #[test]
    fn test_identity_guard_and_force_kill_child() {
        assert!(is_protected_pid(0));
        assert!(is_protected_pid(1));
        assert!(is_protected_pid(std::process::id()));
        #[cfg(target_os = "windows")]
        assert!(is_protected_pid(4));

        let mut child = spawn_test_child();
        let child_pid = child.id();
        let mut sys = System::new();
        refresh_processes_for_inspection(&mut sys, false);

        let mut identities = snapshot_process_identities(&sys, &[0, 1, std::process::id(), child_pid]);
        assert!(!identities.contains_key(&0));
        assert!(!identities.contains_key(&1));
        assert!(!identities.contains_key(&std::process::id()));
        let child_id = identities.get(&child_pid).cloned().expect("child identity captured");

        // Exact identity matches while live
        let kept = retain_live_identities(&sys, identities.clone());
        assert!(kept.contains_key(&child_pid));

        // Recycled PID with different start_time or process name must be dropped
        identities.insert(
            child_pid,
            ProcessIdentity {
                start_time: child_id.start_time.saturating_add(1),
                name_low: child_id.name_low.clone(),
            },
        );
        assert!(!retain_live_identities(&sys, identities.clone()).contains_key(&child_pid));

        identities.insert(
            child_pid,
            ProcessIdentity {
                start_time: child_id.start_time,
                name_low: "unrelated.exe".into(),
            },
        );
        assert!(!retain_live_identities(&sys, identities).contains_key(&child_pid));

        // Force kill and verify post-exit retention is empty
        assert!(force_kill_pid(child_pid));
        let _ = child.wait();
        refresh_processes_for_inspection(&mut sys, false);
        let mut after_map = HashMap::new();
        after_map.insert(child_pid, child_id);
        assert!(retain_live_identities(&sys, after_map).is_empty());
    }

    #[test]
    fn test_force_stop_target_kills_seeded_child() {
        let mut child = spawn_test_child();
        let child_pid = child.id();

        let mut sys = System::new();
        refresh_processes_for_inspection(&mut sys, false);
        let seeds = snapshot_process_identities(&sys, &[child_pid]);
        assert!(seeds.contains_key(&child_pid));

        let empty_spec = TargetAppSpec {
            id: "test-seeded".into(),
            name: "Test".into(),
            name_en: "Test".into(),
            builtin: false,
            exe_path: String::new(),
            proxy_enabled: false,
            probe_urls: vec![],
            child_process_patterns: vec![],
            sidecar_cmdline_regex: String::new(),
            warn_unsaved_on_kill: false,
        };

        assert!(force_stop_target(&empty_spec, &seeds, true).is_ok());
        let _ = child.wait();

        refresh_processes_for_inspection(&mut sys, false);
        assert!(retain_live_identities(&sys, seeds).is_empty());
    }
}
