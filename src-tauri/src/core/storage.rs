use super::models::AppSettings;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub fn resolve_data_dir() -> (PathBuf, bool) {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let portable_data = parent.join("data");
            let portable_flag = parent.join("portable.flag");
            if portable_data.is_dir() || portable_flag.is_file() {
                return (portable_data, true);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(base) = dirs::data_local_dir() {
            return (base.join("Local-AI-Route"), false);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(base) = dirs::data_dir() {
            return (base.join("Local-AI-Route"), false);
        }
    }

    if let Some(base) = dirs::config_dir() {
        (base.join("local-ai-route"), false)
    } else {
        (PathBuf::from("data"), true)
    }
}

pub fn get_settings_file_path() -> PathBuf {
    let (data_dir, _) = resolve_data_dir();
    data_dir.join("settings.json")
}

pub fn load_settings() -> AppSettings {
    let path = get_settings_file_path();
    if path.is_file() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(settings) = serde_json::from_str::<AppSettings>(&data) {
                return settings;
            }
        }
    }
    AppSettings::default()
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let path = get_settings_file_path();
    let parent = path.parent().ok_or_else(|| "Failed to resolve settings parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let serialized = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    
    // Atomic persistence using uniquely named temporary file
    let mut temp = tempfile::Builder::new()
        .prefix(".settings.")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|e| e.to_string())?;

    temp.write_all(serialized.as_bytes()).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(&path).map_err(|e| e.to_string())?;
    Ok(())
}
