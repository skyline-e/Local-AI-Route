#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

pub fn resolve_executable_for_launch(path: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        windows::resolve_executable_for_launch(path)
    }
    #[cfg(target_os = "macos")]
    {
        macos::resolve_executable_for_launch(path)
    }
    #[cfg(target_os = "linux")]
    {
        linux::resolve_executable_for_launch(path)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        path.to_string()
    }
}

pub fn resolve_launch_extra_args(original_exe: &str, resolved_exe: &str) -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        windows::resolve_launch_extra_args(original_exe, resolved_exe)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (original_exe, resolved_exe);
        Vec::new()
    }
}

pub fn discover_target_executable(app_id: &str) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        windows::discover_target_executable(app_id)
    }
    #[cfg(target_os = "macos")]
    {
        macos::discover_target_executable(app_id)
    }
    #[cfg(target_os = "linux")]
    {
        linux::discover_target_executable(app_id)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = app_id;
        None
    }
}

pub fn request_graceful_close(pids: &[u32]) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows::request_graceful_close(pids)
    }
    #[cfg(target_os = "macos")]
    {
        macos::request_graceful_close(pids)
    }
    #[cfg(target_os = "linux")]
    {
        linux::request_graceful_close(pids)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = pids;
        false
    }
}

pub fn is_protected_pid(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    let system_pid = pid <= 4;
    #[cfg(not(target_os = "windows"))]
    let system_pid = pid <= 1;

    system_pid || pid == std::process::id()
}

pub fn force_kill_pid(pid: u32) -> bool {
    if is_protected_pid(pid) {
        return false;
    }

    #[cfg(target_os = "windows")]
    {
        windows::force_kill_pid(pid)
    }
    #[cfg(target_os = "macos")]
    {
        macos::force_kill_pid(pid)
    }
    #[cfg(target_os = "linux")]
    {
        linux::force_kill_pid(pid)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        false
    }
}

pub fn get_process_tcp_connections(
    pids: &std::collections::HashSet<u32>,
    proxy_port: u16,
) -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        windows::get_process_tcp_connections(pids, proxy_port)
    }
    #[cfg(target_os = "macos")]
    {
        macos::get_process_tcp_connections(pids, proxy_port)
    }
    #[cfg(target_os = "linux")]
    {
        linux::get_process_tcp_connections(pids, proxy_port)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (pids, proxy_port);
        Vec::new()
    }
}

pub fn create_desktop_shortcut(
    app_id: &str,
    app_name: &str,
    target_icon_exe: &str,
) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        windows::create_desktop_shortcut(app_id, app_name, target_icon_exe)
    }
    #[cfg(target_os = "macos")]
    {
        macos::create_desktop_shortcut(app_id, app_name, target_icon_exe)
    }
    #[cfg(target_os = "linux")]
    {
        linux::create_desktop_shortcut(app_id, app_name, target_icon_exe)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (app_id, app_name, target_icon_exe);
        Err("Desktop shortcut creation is not supported on this platform".into())
    }
}

pub fn show_native_message_box(title: &str, message: &str) {
    #[cfg(target_os = "windows")]
    {
        windows::show_native_message_box(title, message);
    }
    #[cfg(target_os = "macos")]
    {
        macos::show_native_message_box(title, message);
    }
    #[cfg(target_os = "linux")]
    {
        linux::show_native_message_box(title, message);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        eprintln!("{}: {}", title, message);
    }
}
