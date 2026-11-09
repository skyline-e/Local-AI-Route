#[cfg(target_os = "linux")]
use std::collections::{HashMap, HashSet};
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Command;

#[cfg(target_os = "linux")]
pub fn resolve_executable_for_launch(path: &str) -> String {
    path.to_string()
}

#[cfg(target_os = "linux")]
pub fn discover_target_executable(app_id: &str) -> Option<String> {
    let candidate_names: &[&str] = match app_id {
        "chatgpt" => &["chatgpt", "chatgpt-desktop", "openai-chatgpt"],
        "claude" => &["claude", "claude-desktop", "anthropic-claude"],
        "gemini" => &["gemini", "gemini-desktop", "google-gemini"],
        "antigravity-app" => &["antigravity", "antigravity-desktop"],
        "antigravity-ide" => &["antigravity-ide", "antigravity_ide"],
        "factory" => &["factory-desktop", "factory"],
        _ => return None,
    };

    // 1. Search in PATH
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for &name in candidate_names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate.to_string_lossy().to_string());
                }
            }
        }
    }

    // 2. Common fixed install directories (/usr/bin, /usr/local/bin, /snap/bin, flatpak, ~/.local/bin)
    let home = std::env::var("HOME").unwrap_or_default();
    let mut check_dirs = vec![
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/snap/bin"),
        PathBuf::from("/var/lib/flatpak/exports/bin"),
    ];
    if !home.is_empty() {
        check_dirs.push(PathBuf::from(&home).join(".local/bin"));
        check_dirs.push(PathBuf::from(&home).join(".local/share/flatpak/exports/bin"));
    }

    for dir in &check_dirs {
        for &name in candidate_names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    // 3. Check /opt/<AppDir>/<binary>
    let opt_subdirs: &[&str] = match app_id {
        "chatgpt" => &["ChatGPT", "chatgpt", "chatgpt-desktop"],
        "claude" => &["Claude", "claude", "claude-desktop"],
        "gemini" => &["Gemini", "gemini", "gemini-desktop", "Google Gemini"],
        "antigravity-app" => &["Antigravity", "antigravity"],
        "antigravity-ide" => &["AntigravityIDE", "antigravity-ide", "antigravity_ide"],
        "factory" => &["Factory", "factory-desktop", "factory"],
        _ => &[],
    };

    for sub in opt_subdirs {
        let opt_dir = Path::new("/opt").join(sub);
        for &name in candidate_names {
            let candidate = opt_dir.join(name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    // 4. Search .desktop files in /usr/share/applications and ~/.local/share/applications
    let mut desktop_roots = vec![PathBuf::from("/usr/share/applications")];
    if !home.is_empty() {
        desktop_roots.push(PathBuf::from(&home).join(".local/share/applications"));
    }

    for root in desktop_roots {
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "desktop") {
                    let file_name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase();
                    if candidate_names
                        .iter()
                        .any(|name| file_name.contains(&name.to_lowercase()))
                    {
                        if let Some(exec_path) = extract_exec_from_desktop_file(&path) {
                            if Path::new(&exec_path).is_file() {
                                return Some(exec_path);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn extract_exec_from_desktop_file(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(raw_exec) = trimmed.strip_prefix("Exec=") {
            let mut raw = raw_exec.trim();
            // Skip leading "env VAR=... /path" if present
            if raw.starts_with("env ") {
                raw = raw.strip_prefix("env ").unwrap_or(raw).trim();
                while let Some(space_idx) = raw.find(' ') {
                    let token = &raw[..space_idx];
                    if token.contains('=') {
                        raw = raw[space_idx..].trim();
                    } else {
                        break;
                    }
                }
            }

            if raw.starts_with('"') {
                if let Some(end_idx) = raw[1..].find('"') {
                    return Some(raw[1..=end_idx].to_string());
                }
            } else if let Some(first) = raw.split_whitespace().next() {
                return Some(first.trim_matches('"').to_string());
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
pub fn get_process_tcp_connections(pids: &HashSet<u32>, proxy_port: u16) -> Vec<String> {
    if pids.is_empty() {
        return Vec::new();
    }

    // Step 1: Scan /proc/<pid>/fd/ for socket inodes belonging to our target process tree
    let mut inode_to_pid: HashMap<u64, u32> = HashMap::new();
    for &pid in pids {
        let fd_dir = format!("/proc/{}/fd", pid);
        if let Ok(entries) = fs::read_dir(fd_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                if let Ok(target) = fs::read_link(entry.path()) {
                    let s = target.to_string_lossy();
                    if s.starts_with("socket:[") && s.ends_with(']') {
                        if let Ok(inode) = s["socket:[".len()..s.len() - 1].parse::<u64>() {
                            inode_to_pid.insert(inode, pid);
                        }
                    }
                }
            }
        }
    }

    let mut results = Vec::new();

    // Step 2: Parse /proc/net/tcp and /proc/net/tcp6
    for proc_net_file in &["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(content) = fs::read_to_string(proc_net_file) {
            for line in content.lines().skip(1) {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.len() < 10 {
                    continue;
                }

                // st: connection state, 01 is TCP_ESTABLISHED
                let state = fields[3];
                if state != "01" {
                    continue;
                }

                if let Ok(inode) = fields[9].parse::<u64>() {
                    if let Some(&pid) = inode_to_pid.get(&inode) {
                        if let (Some((local_ip, local_port)), Some((rem_ip, rem_port))) = (
                            parse_proc_net_address(fields[1]),
                            parse_proc_net_address(fields[2]),
                        ) {
                            let is_proxy = rem_port == proxy_port;
                            let proxy_tag = if is_proxy { " [指向代理端口]" } else { "" };
                            results.push(format!(
                                "PID: {:<5} | TCP {:>15}:{} -> {:>15}:{} [ESTABLISHED]{}",
                                pid, local_ip, local_port, rem_ip, rem_port, proxy_tag
                            ));
                        }
                    }
                }
            }
        }
    }

    // Step 3: Fallback using ss -ntp if /proc yielded nothing
    if results.is_empty() {
        if let Ok(output) = Command::new("ss").args(["-ntp"]).output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines().skip(1) {
                    for &pid in pids {
                        let pid_pat = format!("pid={}", pid);
                        if line.contains(&pid_pat) {
                            let parts: Vec<&str> = line.split_whitespace().collect();
                            if parts.len() >= 5 {
                                let local = parts[3];
                                let peer = parts[4];
                                let is_proxy = peer.ends_with(&format!(":{}", proxy_port));
                                let proxy_tag = if is_proxy { " [指向代理端口]" } else { "" };
                                results.push(format!(
                                    "PID: {:<5} | TCP {:>21} -> {:>21} [ESTABLISHED]{}",
                                    pid, local, peer, proxy_tag
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    results
}

#[cfg(target_os = "linux")]
fn parse_proc_net_address(addr_str: &str) -> Option<(String, u16)> {
    let parts: Vec<&str> = addr_str.split(':').collect();
    if parts.len() != 2 {
        return None;
    }

    let ip_hex = parts[0];
    let port = u16::from_str_radix(parts[1], 16).ok()?;

    if ip_hex.len() == 8 {
        // IPv4 in little-endian hex (e.g. 0100007F -> 127.0.0.1)
        let num = u32::from_str_radix(ip_hex, 16).ok()?;
        let b1 = (num & 0xFF) as u8;
        let b2 = ((num >> 8) & 0xFF) as u8;
        let b3 = ((num >> 16) & 0xFF) as u8;
        let b4 = ((num >> 24) & 0xFF) as u8;
        Some((format!("{}.{}.{}.{}", b1, b2, b3, b4), port))
    } else if ip_hex.len() == 32 {
        // IPv6 in 4 x 32-bit little-endian words
        let mut octets = [0u8; 16];
        for i in 0..4 {
            let slice = &ip_hex[i * 8..(i + 1) * 8];
            let word = u32::from_str_radix(slice, 16).ok()?;
            octets[i * 4] = (word & 0xFF) as u8;
            octets[i * 4 + 1] = ((word >> 8) & 0xFF) as u8;
            octets[i * 4 + 2] = ((word >> 16) & 0xFF) as u8;
            octets[i * 4 + 3] = ((word >> 24) & 0xFF) as u8;
        }
        let ip6 = std::net::Ipv6Addr::from(octets);
        Some((format!("[{}]", ip6), port))
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
pub fn create_desktop_shortcut(
    app_id: &str,
    app_name: &str,
    target_icon_exe: &str,
) -> Result<String, String> {
    let desktop_dir = dirs::desktop_dir().ok_or_else(|| "无法获取桌面目录路径".to_string())?;
    let current_exe = std::env::current_exe().map_err(|e| format!("获取当前程序路径失败: {}", e))?;

    let file_name = format!("local-ai-route-{}.desktop", app_id);
    let shortcut_path = desktop_dir.join(file_name);

    // FreeDesktop Icon field: prefer image files (.png/.svg) or theme icon names.
    // If target_icon_exe is an ELF binary, look for adjacent .png/.svg or fallback to app_id / network icon.
    let icon_field = if !target_icon_exe.is_empty() {
        let p = Path::new(target_icon_exe);
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext == "png" || ext == "svg" || ext == "ico" {
            target_icon_exe.to_string()
        } else {
            let parent = p.parent().unwrap_or(Path::new(""));
            let candidates = [
                parent.join(format!("{}.png", app_id)),
                parent.join("icon.png"),
                parent.join(format!("{}.svg", app_id)),
                parent.join("resources").join("icon.png"),
            ];
            candidates
                .iter()
                .find(|c| c.is_file())
                .map(|c| c.to_string_lossy().to_string())
                .unwrap_or_else(|| app_id.to_string())
        }
    } else {
        "network-wired".to_string()
    };

    let content = format!(
        "[Desktop Entry]\n\
         Version=1.0\n\
         Type=Application\n\
         Name={app_name} (代理)\n\
         Comment=通过 Local-AI-Route 启动 {app_name} 并自动注入本地代理\n\
         Exec=\"{}\" --launch {app_id}\n\
         Icon={icon_field}\n\
         Terminal=false\n\
         Categories=Utility;Network;\n",
        current_exe.to_string_lossy()
    );

    fs::write(&shortcut_path, content).map_err(|e| format!("写入桌面快捷方式失败: {}", e))?;

    let mut perms = fs::metadata(&shortcut_path)
        .map_err(|e| e.to_string())?
        .permissions();
    perms.set_mode(0o755);
    let _ = fs::set_permissions(&shortcut_path, perms);

    Ok(shortcut_path.to_string_lossy().to_string())
}

#[cfg(target_os = "linux")]
pub fn show_native_message_box(title: &str, message: &str) {
    if Command::new("zenity")
        .args(["--warning", &format!("--title={}", title), &format!("--text={}", message)])
        .spawn()
        .is_ok()
    {
        return;
    }

    if Command::new("kdialog")
        .args(["--title", title, "--sorry", message])
        .spawn()
        .is_ok()
    {
        return;
    }

    eprintln!("{}: {}", title, message);
}
