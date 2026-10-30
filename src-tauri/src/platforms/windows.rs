#[cfg(target_os = "windows")]
use std::path::Path;
#[cfg(target_os = "windows")]
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
#[cfg(target_os = "windows")]
use winreg::RegKey;

#[cfg(target_os = "windows")]
pub fn discover_target_executable(app_id: &str) -> Option<String> {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let prog_files = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());

    match app_id {
        "chatgpt" => {
            // 1. Check Store Execution Alias
            if !local_app_data.is_empty() {
                let alias = Path::new(&local_app_data)
                    .join("Microsoft")
                    .join("WindowsApps")
                    .join("ChatGPT.exe");
                if alias.is_file() {
                    return Some(alias.to_string_lossy().to_string());
                }
            }

            // 2. Check Appx Repository in HKCU
            if let Ok(hkcu) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
                r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages",
                KEY_READ,
            ) {
                for pkg_name in hkcu.enum_keys().filter_map(|r| r.ok()) {
                    let low = pkg_name.to_lowercase();
                    if low.starts_with("openai.chatgpt") || low.starts_with("openai.codex") {
                        if let Ok(pkg_key) = hkcu.open_subkey_with_flags(&pkg_name, KEY_READ) {
                            if let Ok(root) = pkg_key.get_value::<String, _>("PackageRootFolder") {
                                let c1 = Path::new(&root).join("app").join("ChatGPT.exe");
                                if c1.is_file() {
                                    return Some(c1.to_string_lossy().to_string());
                                }
                                let c2 = Path::new(&root).join("ChatGPT.exe");
                                if c2.is_file() {
                                    return Some(c2.to_string_lossy().to_string());
                                }
                            }
                        }
                    }
                }
            }

            // 3. Fallback standard paths
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

                        // Try DisplayIcon
                        if let Ok(icon) = key.get_value::<String, _>("DisplayIcon") {
                            let cleaned = icon.split(',').next().unwrap_or("").trim().trim_matches('"');
                            if Path::new(cleaned).is_file() && cleaned.to_lowercase().ends_with(".exe") {
                                return Some(cleaned.to_string());
                            }
                        }

                        // Try InstallLocation
                        if let Ok(loc) = key.get_value::<String, _>("InstallLocation") {
                            let cleaned = loc.trim().trim_matches('"');
                            let candidate = Path::new(cleaned).join(fallback_exe);
                            if candidate.is_file() {
                                return Some(candidate.to_string_lossy().to_string());
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

        let mut buffer: Vec<u8> = vec![0; size as usize];
        let ret = GetExtendedTcpTable(
            Some(buffer.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );

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
    let link_path = desktop.join(format!("{} (Local-AI-Route).lnk", app_name));

    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;

    let mut sl = mslnk::ShellLink::new(&current_exe).map_err(|e| e.to_string())?;
    sl.set_arguments(Some(format!("--launch {}", app_id)));
    if !target_icon_exe.is_empty() && Path::new(target_icon_exe).is_file() {
        sl.set_icon_location(Some(target_icon_exe.to_string()));
    }
    sl.create_lnk(&link_path).map_err(|e| e.to_string())?;

    Ok(link_path.to_string_lossy().to_string())
}
