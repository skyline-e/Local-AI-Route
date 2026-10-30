#[cfg(target_os = "macos")]
use std::path::Path;

#[cfg(target_os = "macos")]
pub fn discover_target_executable(app_id: &str) -> Option<String> {
    let bundle_name = match app_id {
        "chatgpt" => "ChatGPT.app",
        "antigravity-app" => "Antigravity.app",
        "antigravity-ide" => "Antigravity IDE.app",
        _ => return None,
    };

    let roots = ["/Applications", &format!("{}/Applications", std::env::var("HOME").unwrap_or_default())];
    for root in roots {
        let bundle = Path::new(root).join(bundle_name);
        if bundle.is_dir() {
            let macos_dir = bundle.join("Contents").join("MacOS");
            if let Ok(entries) = std::fs::read_dir(macos_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_file() {
                        return Some(path.to_string_lossy().to_string());
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
        unsafe {
            if libc::kill(pid as i32, 15) == 0 {
                sent = true;
            }
        }
    }
    sent
}

#[cfg(target_os = "macos")]
pub fn get_process_tcp_connections(_pids: &std::collections::HashSet<u32>, _proxy_port: u16) -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "macos")]
pub fn show_native_message_box(title: &str, message: &str) {
    eprintln!("{}: {}", title, message);
}
