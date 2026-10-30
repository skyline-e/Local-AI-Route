pub mod macos;
pub mod windows;

pub fn discover_target_executable(app_id: &str) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        windows::discover_target_executable(app_id)
    }
    #[cfg(target_os = "macos")]
    {
        macos::discover_target_executable(app_id)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
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
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

pub fn get_process_tcp_connections(pids: &std::collections::HashSet<u32>, proxy_port: u16) -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        windows::get_process_tcp_connections(pids, proxy_port)
    }
    #[cfg(target_os = "macos")]
    {
        macos::get_process_tcp_connections(pids, proxy_port)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (pids, proxy_port);
        Vec::new()
    }
}

pub fn create_desktop_shortcut(app_id: &str, app_name: &str, target_icon_exe: &str) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        windows::create_desktop_shortcut(app_id, app_name, target_icon_exe)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Desktop shortcut creation is currently supported on Windows".into())
    }
}
