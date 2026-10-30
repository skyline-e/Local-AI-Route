#[cfg(target_os = "macos")]
use std::collections::HashSet;
#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "macos")]
pub fn resolve_executable_for_launch(path_str: &str) -> String {
    let p = Path::new(path_str);
    if p.is_dir()
        && (p.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("app"))
            || p.join("Contents/MacOS").is_dir())
    {
        // 1. Try reading CFBundleExecutable from Info.plist
        let plist_path = p.join("Contents").join("Info.plist");
        if let Ok(content) = fs::read_to_string(&plist_path) {
            static PLIST_EXE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            let re = PLIST_EXE_RE.get_or_init(|| {
                regex::Regex::new(r"<key>CFBundleExecutable</key>\s*<string>([^<]+)</string>")
                    .expect("valid regex")
            });
            if let Some(caps) = re.captures(&content) {
                if let Some(m) = caps.get(1) {
                    let exe = p.join("Contents").join("MacOS").join(m.as_str().trim());
                    if exe.is_file() {
                        return exe.to_string_lossy().to_string();
                    }
                }
            }
        }

        // 2. Fallback: take the first executable file in Contents/MacOS
        let macos_dir = p.join("Contents").join("MacOS");
        if let Ok(entries) = fs::read_dir(&macos_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let candidate = entry.path();
                if candidate.is_file() {
                    return candidate.to_string_lossy().to_string();
                }
            }
        }
    }
    path_str.to_string()
}

#[cfg(target_os = "macos")]
pub fn discover_target_executable(app_id: &str) -> Option<String> {
    let bundle_candidates: &[&str] = match app_id {
        "chatgpt" => &["ChatGPT.app"],
        "claude" => &["Claude.app", "Claude Desktop.app"],
        "antigravity-app" => &["Antigravity.app"],
        "antigravity-ide" => &["Antigravity IDE.app", "AntigravityIDE.app"],
        _ => return None,
    };

    let home = std::env::var("HOME").unwrap_or_default();
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if !home.is_empty() {
        roots.push(PathBuf::from(&home).join("Applications"));
    }

    for root in &roots {
        for &bundle_name in bundle_candidates {
            let bundle = root.join(bundle_name);
            if bundle.is_dir() {
                let resolved = resolve_executable_for_launch(&bundle.to_string_lossy());
                if Path::new(&resolved).is_file() {
                    return Some(resolved);
                }
            }
        }
    }

    // Fallback: search via macOS Spotlight mdfind
    for &bundle_name in bundle_candidates {
        if let Ok(output) = Command::new("mdfind").args(["-name", bundle_name]).output() {
            if output.status.success() {
                let out_str = String::from_utf8_lossy(&output.stdout);
                for line in out_str.lines() {
                    let trimmed = line.trim();
                    if trimmed.ends_with(bundle_name) && Path::new(trimmed).is_dir() {
                        let resolved = resolve_executable_for_launch(trimmed);
                        if Path::new(&resolved).is_file() {
                            return Some(resolved);
                        }
                    }
                }
            }
        }
    }

    None
}

#[cfg(target_os = "macos")]
pub fn request_graceful_close(pids: &[u32]) -> bool {
    let mut sent = false;
    for &pid in pids {
        if pid <= 1 {
            continue;
        }
        unsafe {
            if libc::kill(pid as i32, 15) == 0 {
                sent = true;
            }
        }
    }
    sent
}

#[cfg(target_os = "macos")]
pub fn get_process_tcp_connections(pids: &HashSet<u32>, proxy_port: u16) -> Vec<String> {
    if pids.is_empty() {
        return Vec::new();
    }

    let pid_csv = pids
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let output = match Command::new("lsof")
        .args(["-n", "-P", "-iTCP", "-sTCP:ESTABLISHED", "-a", "-p", &pid_csv])
        .output()
    {
        Ok(out) => out,
        Err(_) => return Vec::new(),
    };

    if !output.status.success() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut records = Vec::new();

    for line in text.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        // lsof output format: COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME
        if parts.len() < 8 {
            continue;
        }

        // Connection endpoint string is in the last column
        let name = match parts.last() {
            Some(&n) if n.contains("->") => n,
            _ => continue,
        };

        let (local, remote) = match name.split_once("->") {
            Some((l, r)) => (l, r),
            None => continue,
        };

        // Locate PID token belonging to our target pids set (robust against process names with spaces)
        let pid = match parts.iter().find_map(|s| s.parse::<u32>().ok().filter(|p| pids.contains(p))) {
            Some(p) => p,
            None => continue,
        };

        let is_proxy = remote.ends_with(&format!(":{}", proxy_port));
        let proxy_tag = if is_proxy { " [指向代理端口]" } else { "" };

        records.push(format!(
            "PID: {:<5} | TCP {:>21} -> {:>21} [ESTABLISHED]{}",
            pid, local, remote, proxy_tag
        ));
    }

    records
}

#[cfg(target_os = "macos")]
pub fn create_desktop_shortcut(
    app_id: &str,
    app_name: &str,
    target_icon_exe: &str,
) -> Result<String, String> {
    let desktop = dirs::desktop_dir().ok_or_else(|| "无法获取桌面目录路径".to_string())?;
    let current_exe = std::env::current_exe().map_err(|e| format!("获取当前程序路径失败: {}", e))?;

    let app_bundle_name = format!("{} (代理).app", app_name);
    let target_app_path = desktop.join(&app_bundle_name);

    if target_app_path.exists() {
        let _ = fs::remove_dir_all(&target_app_path);
    }

    let exe_escaped = current_exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let app_escaped = app_id.replace('\\', "\\\\").replace('"', "\\\"");

    let script = format!(
        "do shell script quoted form of \"{}\" & \" --launch \" & quoted form of \"{}\" & \" > /dev/null 2>&1 &\"",
        exe_escaped, app_escaped
    );

    let osacompile_res = Command::new("osacompile")
        .args(["-e", &script, "-o", &target_app_path.to_string_lossy()])
        .output();

    if let Ok(out) = osacompile_res {
        if out.status.success() {
            if !target_icon_exe.is_empty() {
                let target_path = Path::new(target_icon_exe);
                let bundle_dir = if target_path.to_string_lossy().contains(".app") {
                    let mut curr = target_path;
                    while let Some(parent) = curr.parent() {
                        if curr.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("app")) {
                            break;
                        }
                        curr = parent;
                    }
                    Some(curr)
                } else {
                    None
                };

                if let Some(bundle) = bundle_dir {
                    let res_dir = bundle.join("Contents").join("Resources");
                    if let Ok(entries) = fs::read_dir(res_dir) {
                        for entry in entries.filter_map(|e| e.ok()) {
                            let p = entry.path();
                            if p.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("icns")) {
                                let dest_icon = target_app_path.join("Contents/Resources/applet.icns");
                                let _ = fs::copy(&p, dest_icon);
                                break;
                            }
                        }
                    }
                }
            }
            return Ok(target_app_path.to_string_lossy().to_string());
        }
    }

    // Fallback: generate a .command executable shell script
    let cmd_name = format!("{} (代理).command", app_name);
    let cmd_path = desktop.join(cmd_name);
    let script_content = format!(
        "#!/bin/bash\nexec \"{}\" --launch \"{}\" >/dev/null 2>&1 &\n",
        current_exe.to_string_lossy(),
        app_id
    );

    fs::write(&cmd_path, script_content).map_err(|e| format!("写入快捷方式脚本失败: {}", e))?;

    let mut perms = fs::metadata(&cmd_path).map_err(|e| e.to_string())?.permissions();
    perms.set_mode(0o755);
    let _ = fs::set_permissions(&cmd_path, perms);

    Ok(cmd_path.to_string_lossy().to_string())
}

#[cfg(target_os = "macos")]
pub fn show_native_message_box(title: &str, message: &str) {
    if Command::new("osascript")
        .args([
            "-e",
            "on run {alertTitle, alertMessage}",
            "-e",
            "display alert alertTitle message alertMessage as critical buttons {\"好\"} default button \"好\"",
            "-e",
            "end run",
            "--",
            title,
            message,
        ])
        .spawn()
        .is_ok()
    {
        return;
    }

    eprintln!("{}: {}", title, message);
}
