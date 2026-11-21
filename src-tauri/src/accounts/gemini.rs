use std::fs;
use std::path::{Path, PathBuf};

fn candidate_gemini_profile_dirs() -> Vec<PathBuf> {
    let mut dirs_list = Vec::new();
    if let Some(config_dir) = dirs::config_dir() {
        for sub in &["gemini", "Gemini", "Google/Gemini", "google-gemini"] {
            let p = config_dir.join(sub);
            if p.is_dir() {
                dirs_list.push(p);
            }
        }
    }

    #[cfg(target_os = "windows")]
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let local_p = Path::new(&local);
        for sub in &[
            "Google/Gemini/User Data",
            "Gemini/User Data",
            "google-gemini",
        ] {
            let p = local_p.join(sub);
            if p.is_dir() && !dirs_list.contains(&p) {
                dirs_list.push(p);
            }
        }
    }

    dirs_list
}

fn clear_profile_dir_collect(
    profile_dir: &Path,
    cleared: &mut Vec<String>,
    failures: &mut Vec<String>,
) {
    if !profile_dir.is_dir() {
        return;
    }

    let mut targets = vec![profile_dir.to_path_buf()];
    let default_subdir = profile_dir.join("Default");
    if default_subdir.is_dir() {
        targets.push(default_subdir);
    }

    // Electron apps may isolate Google auth cookies inside Partitions/<partition_name>/
    for base in targets.clone() {
        let partitions_dir = base.join("Partitions");
        if let Ok(entries) = fs::read_dir(&partitions_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    targets.push(p);
                }
            }
        }
    }

    for target in targets {
        super::remove_chromium_cookie_files_strict(&target, cleared, failures);
        for storage_dir_name in &["Local Storage", "Session Storage"] {
            super::remove_dir_all_strict(&target.join(storage_dir_name), cleared, failures);
        }
    }
}

/// Clears session cookies and web storage inside a single Gemini Chromium/Electron profile directory
/// while preserving `Preferences` (which stores Local-AI-Route's proxy settings).
#[cfg(test)]
pub(crate) fn clear_profile_login_state_in_dir(profile_dir: &Path) -> Result<Vec<String>, String> {
    let mut cleared = Vec::new();
    let mut failures = Vec::new();
    clear_profile_dir_collect(profile_dir, &mut cleared, &mut failures);
    super::finalize_clear_result(cleared, failures)
}

pub fn clear_live_credentials() -> Result<Vec<String>, String> {
    let mut cleared = Vec::new();
    let mut failures = Vec::new();
    for dir in candidate_gemini_profile_dirs() {
        clear_profile_dir_collect(&dir, &mut cleared, &mut failures);
    }
    super::finalize_clear_result(cleared, failures)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clear_profile_preserves_preferences_and_removes_cookies_and_storage() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Preferences must be preserved (stores Gemini proxy config)
        let pref = root.join("Preferences");
        fs::write(&pref, r#"{"proxy":{"mode":"fixed_servers"}}"#).unwrap();

        // Cookies in Network/, Default/Network/, and Partitions/main/Network/
        let net = root.join("Network");
        fs::create_dir_all(&net).unwrap();
        fs::write(net.join("Cookies"), b"cookie-data").unwrap();
        fs::write(net.join("Cookies-journal"), b"journal").unwrap();

        let local_storage = root.join("Local Storage");
        fs::create_dir_all(&local_storage).unwrap();
        fs::write(local_storage.join("000003.log"), b"ls").unwrap();

        let session_storage = root.join("Session Storage");
        fs::create_dir_all(&session_storage).unwrap();
        fs::write(session_storage.join("CURRENT"), b"ss").unwrap();

        let part_net = root.join("Partitions").join("google").join("Network");
        fs::create_dir_all(&part_net).unwrap();
        fs::write(part_net.join("Cookies"), b"part-cookie").unwrap();

        let cleared = clear_profile_login_state_in_dir(root).expect("clear ok");

        assert!(pref.is_file(), "Preferences should be preserved");
        assert!(!net.join("Cookies").exists());
        assert!(!net.join("Cookies-journal").exists());
        assert!(!local_storage.exists());
        assert!(!session_storage.exists());
        assert!(!part_net.join("Cookies").exists());
        assert!(cleared.len() >= 5);
    }
}
