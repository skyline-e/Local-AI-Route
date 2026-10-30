#[cfg(target_os = "windows")]
use std::path::Path;
#[cfg(target_os = "windows")]
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
#[cfg(target_os = "windows")]
use winreg::RegKey;

#[cfg(target_os = "windows")]
pub fn resolve_executable_for_launch(path: &str) -> String {
    path.to_string()
}

#[cfg(target_os = "windows")]
pub fn discover_target_executable(app_id: &str) -> Option<String> {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let prog_files = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    let prog_files_x86 = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());

    match app_id {
        "chatgpt" => {
            // 1. Check Appx / MSIX Repository in HKCU (prefer real package binary over alias)
            if let Some(found) = find_latest_appx_exe(&["openai.chatgpt", "openai.codex"], "ChatGPT.exe") {
                return Some(found);
            }

            // 2. Check standard paths
            if !local_app_data.is_empty() {
                let c = Path::new(&local_app_data)
                    .join("Programs")
                    .join("ChatGPT")
                    .join("ChatGPT.exe");
                if c.is_file() {
                    return Some(c.to_string_lossy().to_string());
                }
            }
            let c = Path::new(&prog_files).join("ChatGPT").join("ChatGPT.exe");
            if c.is_file() {
                return Some(c.to_string_lossy().to_string());
            }

            // 3. Fallback to Store Execution Alias
            if !local_app_data.is_empty() {
                for alias_name in &["ChatGPT.exe", "chatgpt-desktop.exe"] {
                    let alias = Path::new(&local_app_data)
                        .join("Microsoft")
                        .join("WindowsApps")
                        .join(alias_name);
                    if is_executable_or_alias(&alias) {
                        return Some(alias.to_string_lossy().to_string());
                    }
                }
            }
        }
        "claude" => {
            // 1. Check Appx / MSIX Repository in HKCU (prefer real package binary over alias for direct flag injection & icon)
            if let Some(found) = find_latest_appx_exe(&["claude_", "anthropic."], "Claude.exe") {
                return Some(found);
            }

            // 2. Check Squirrel / Win32 install in %LOCALAPPDATA%\AnthropicClaude\app-*\Claude.exe
            if !local_app_data.is_empty() {
                let squirrel_root = Path::new(&local_app_data).join("AnthropicClaude");
                if let Some(found) = find_latest_squirrel_exe(&squirrel_root, "Claude.exe") {
                    return Some(found);
                }
                let squirrel_stub = squirrel_root.join("Claude.exe");
                if squirrel_stub.is_file() {
                    return Some(squirrel_stub.to_string_lossy().to_string());
                }

                // 3. Check standard LocalAppData Programs paths
                for sub in &[
                    r"Programs\Claude\Claude.exe",
                    r"Programs\Claude Desktop\Claude.exe",
                    r"Programs\claude-desktop\Claude.exe",
                ] {
                    let candidate = Path::new(&local_app_data).join(sub);
                    if candidate.is_file() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }

            // 4. Check ProgramFiles / ProgramFiles(x86) paths
            for base in &[&prog_files, &prog_files_x86] {
                if base.is_empty() {
                    continue;
                }
                for sub in &[
                    r"Anthropic\Claude\Claude.exe",
                    r"Claude\Claude.exe",
                    r"Claude Desktop\Claude.exe",
                ] {
                    let candidate = Path::new(base).join(sub);
                    if candidate.is_file() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }

            // 5. Check Uninstall registry
            if let Some(path) = search_uninstall_registry("claude", false, "Claude.exe") {
                return Some(path);
            }

            // 6. Fallback to Windows Store Execution Alias
            if !local_app_data.is_empty() {
                let alias = Path::new(&local_app_data)
                    .join("Microsoft")
                    .join("WindowsApps")
                    .join("Claude.exe");
                if is_executable_or_alias(&alias) {
                    return Some(alias.to_string_lossy().to_string());
                }
            }
        }
        "antigravity-app" => {
            // Search Uninstall keys
            if let Some(path) = search_uninstall_registry("antigravity", false, "Antigravity.exe") {
                return Some(path);
            }
            if !local_app_data.is_empty() {
                let c = Path::new(&local_app_data)
                    .join("Programs")
                    .join("Antigravity")
                    .join("Antigravity.exe");
                if c.is_file() {
                    return Some(c.to_string_lossy().to_string());
                }
            }
        }
        "antigravity-ide" => {
            if let Some(path) = search_uninstall_registry("antigravity ide", true, "Antigravity IDE.exe") {
                return Some(path);
            }
            if !local_app_data.is_empty() {
                let c = Path::new(&local_app_data)
                    .join("Programs")
                    .join("Antigravity IDE")
                    .join("Antigravity IDE.exe");
                if c.is_file() {
                    return Some(c.to_string_lossy().to_string());
                }
            }
        }
        _ => {}
    }
    None
}

#[cfg(target_os = "windows")]
fn is_executable_or_alias(path: &Path) -> bool {
    path.is_file() || std::fs::symlink_metadata(path).map(|m| !m.is_dir()).unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn find_latest_appx_exe(prefixes: &[&str], exe_name: &str) -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages",
            KEY_READ,
        )
        .ok()?;

    let mut candidates: Vec<(std::time::SystemTime, String)> = Vec::new();

    for pkg_name in hkcu.enum_keys().filter_map(|r| r.ok()) {
        let low = pkg_name.to_lowercase();
        if !prefixes.iter().any(|p| low.starts_with(p)) {
            continue;
        }
        if let Ok(pkg_key) = hkcu.open_subkey_with_flags(&pkg_name, KEY_READ) {
            if let Ok(root) = pkg_key.get_value::<String, _>("PackageRootFolder") {
                let root_path = Path::new(&root);
                for candidate in [root_path.join("app").join(exe_name), root_path.join(exe_name)] {
                    if candidate.is_file() {
                        let modified = candidate
                            .metadata()
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        candidates.push((modified, candidate.to_string_lossy().to_string()));
                        break;
                    }
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    candidates.into_iter().next().map(|(_, path)| path)
}

#[cfg(target_os = "windows")]
fn find_latest_squirrel_exe(root_dir: &Path, exe_name: &str) -> Option<String> {
    let entries = std::fs::read_dir(root_dir).ok()?;
    let mut candidates: Vec<(std::time::SystemTime, String)> = Vec::new();

    for entry in entries.filter_map(|e| e.ok()) {
        let file_name = entry.file_name().to_string_lossy().to_lowercase();
        if file_name.starts_with("app-") && entry.path().is_dir() {
            let exe_path = entry.path().join(exe_name);
            if exe_path.is_file() {
                let modified = exe_path
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                candidates.push((modified, exe_path.to_string_lossy().to_string()));
            }
        }
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    candidates.into_iter().next().map(|(_, path)| path)
}

#[cfg(target_os = "windows")]
fn search_uninstall_registry(pattern: &str, require_ide: bool, fallback_exe: &str) -> Option<String> {
    let hives = [
        (HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];

    for (hive, subkey) in hives {
        if let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(subkey, KEY_READ) {
            for item in root.enum_keys().filter_map(|r| r.ok()) {
                if let Ok(key) = root.open_subkey_with_flags(&item, KEY_READ) {
                    let disp_name = key.get_value::<String, _>("DisplayName").unwrap_or_default();
                    let disp_name_low = disp_name.to_lowercase();

                    if disp_name_low.contains(pattern) {
                        let is_ide = disp_name_low.contains("ide");
                        if require_ide != is_ide {
                            continue;
                        }

                        // Try InstallLocation first (including Squirrel app-* and app\ subdirectories)
                        if let Ok(loc) = key.get_value::<String, _>("InstallLocation") {
                            let cleaned = loc.trim().trim_matches('"');
                            if !cleaned.is_empty() {
                                let loc_path = Path::new(cleaned);
                                if let Some(squirrel_exe) = find_latest_squirrel_exe(loc_path, fallback_exe) {
                                    return Some(squirrel_exe);
                                }
                                let app_sub = loc_path.join("app").join(fallback_exe);
                                if app_sub.is_file() {
                                    return Some(app_sub.to_string_lossy().to_string());
                                }
                                let candidate = loc_path.join(fallback_exe);
                                if candidate.is_file() {
                                    return Some(candidate.to_string_lossy().to_string());
                                }
                            }
                        }

                        // Try DisplayIcon (excluding Squirrel Update.exe and uninstallers)
                        if let Ok(icon) = key.get_value::<String, _>("DisplayIcon") {
                            let cleaned = icon.split(',').next().unwrap_or("").trim().trim_matches('"');
                            let icon_path = Path::new(cleaned);
                            if let Some(fname) = icon_path.file_name().and_then(|n| n.to_str()) {
                                let fname_low = fname.to_lowercase();
                                if icon_path.is_file()
                                    && fname_low.ends_with(".exe")
                                    && !matches!(fname_low.as_str(), "update.exe" | "uninstall.exe" | "unins000.exe")
                                {
                                    return Some(cleaned.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
pub fn request_graceful_close(pids: &[u32]) -> bool {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, GA_ROOT, WM_CLOSE,
    };

    struct Context<'a> {
        pids: &'a [u32],
        closed_any: bool,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Context);
        let mut proc_id: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut proc_id));
        if ctx.pids.contains(&proc_id) {
            let is_visible = IsWindowVisible(hwnd).as_bool();
            let is_root = GetAncestor(hwnd, GA_ROOT) == hwnd;
            if is_visible || is_root {
                let _ = PostMessageW(hwnd, WM_CLOSE, None, None);
                ctx.closed_any = true;
            }
        }
        BOOL(1)
    }

    let mut ctx = Context {
        pids,
        closed_any: false,
    };

    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.closed_any
}

#[cfg(target_os = "windows")]
pub fn get_process_tcp_connections(pids: &std::collections::HashSet<u32>, proxy_port: u16) -> Vec<String> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
    };
    use windows::Win32::Networking::WinSock::AF_INET;

    let mut records = Vec::new();
    let mut size: u32 = 0;

    unsafe {
        // Query initial table size
        let _ = GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );

        if size == 0 {
            return records;
        }

        // Allocate buffer with headroom to avoid race conditions during socket bursts
        let mut buffer: Vec<u8> = vec![0; ((size as usize * 12) / 10) + 256];
        let mut alloc_size = buffer.len() as u32;

        let mut ret = GetExtendedTcpTable(
            Some(buffer.as_mut_ptr() as *mut _),
            &mut alloc_size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );

        // Retry once if buffer was still insufficient
        if ret == 122 {
            buffer = vec![0; (alloc_size as usize) + 512];
            alloc_size = buffer.len() as u32;
            ret = GetExtendedTcpTable(
                Some(buffer.as_mut_ptr() as *mut _),
                &mut alloc_size,
                false,
                AF_INET.0 as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            );
        }

        if ret == 0 {
            let table = &*(buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
            let entries = std::slice::from_raw_parts(
                table.table.as_ptr(),
                table.dwNumEntries as usize,
            );

            for row in entries {
                let owning_pid = row.dwOwningPid;
                if pids.contains(&owning_pid) {
                    let remote_port = u16::from_be((row.dwRemotePort & 0xFFFF) as u16);
                    let remote_ip = std::net::Ipv4Addr::from(row.dwRemoteAddr.to_ne_bytes());
                    let local_port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);

                    let state_str = match row.dwState {
                        1 => "CLOSED",
                        2 => "LISTEN",
                        3 => "SYN_SENT",
                        4 => "SYN_RCVD",
                        5 => "ESTABLISHED",
                        6 => "FIN_WAIT1",
                        7 => "FIN_WAIT2",
                        8 => "CLOSE_WAIT",
                        9 => "CLOSING",
                        10 => "LAST_ACK",
                        11 => "TIME_WAIT",
                        12 => "DELETE_TCB",
                        _ => "UNKNOWN",
                    };

                    if remote_port == proxy_port || state_str == "ESTABLISHED" {
                        let is_proxy = if remote_port == proxy_port { " [指向代理端口]" } else { "" };
                        records.push(format!(
                            "✔ PID {} (端口 :{}) -> {}:{}: {}{}",
                            owning_pid, local_port, remote_ip, remote_port, state_str, is_proxy
                        ));
                    }
                }
            }
        }
    }
    records
}

#[cfg(target_os = "windows")]
pub fn create_desktop_shortcut(app_id: &str, app_name: &str, target_icon_exe: &str) -> Result<String, String> {
    let desktop = dirs::desktop_dir().ok_or_else(|| "Could not find desktop directory".to_string())?;

    // Filter invalid Windows filename characters to avoid OS error 123
    let clean_app_name: String = app_name
        .chars()
        .map(|c| match c {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' | '/' | '\\' => '_',
            other => other,
        })
        .collect();

    let link_path = desktop.join(format!("{} (Local-AI-Route).lnk", clean_app_name));

    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;

    let mut sl = mslnk::ShellLink::new(&current_exe).map_err(|e| e.to_string())?;
    sl.set_arguments(Some(format!("--launch {}", app_id)));
    if let Some(parent) = current_exe.parent() {
        sl.set_working_dir(Some(parent.to_string_lossy().to_string()));
    }
    if !target_icon_exe.is_empty() && Path::new(target_icon_exe).is_file() {
        sl.set_icon_location(Some(target_icon_exe.to_string()));
    }
    sl.create_lnk(&link_path).map_err(|e| e.to_string())?;

    Ok(link_path.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
pub fn show_native_message_box(title: &str, message: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};

    let title_h = HSTRING::from(title);
    let message_h = HSTRING::from(message);
    unsafe {
        let _ = MessageBoxW(None, &message_h, &title_h, MB_OK | MB_ICONWARNING);
    }
}
