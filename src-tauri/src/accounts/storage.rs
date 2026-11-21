use super::models::AccountProfile;
use crate::core::storage::resolve_data_dir;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn get_provider_accounts_dir(provider: &str) -> PathBuf {
    let (data_dir, _) = resolve_data_dir();
    data_dir.join("accounts").join(provider)
}

pub fn get_account_dir(provider: &str, account_id: &str) -> PathBuf {
    get_provider_accounts_dir(provider).join(account_id)
}

pub(crate) fn load_accounts_in_dir(dir: &Path) -> Result<Vec<AccountProfile>, String> {
    let index_file = dir.join("accounts.json");
    if !index_file.is_file() {
        return Ok(Vec::new());
    }

    let data = fs::read_to_string(&index_file).map_err(|e| format!("Failed to read accounts.json: {}", e))?;
    let accounts: Vec<AccountProfile> = serde_json::from_str(&data).unwrap_or_default();
    Ok(accounts)
}

pub fn load_accounts(provider: &str) -> Result<Vec<AccountProfile>, String> {
    let dir = get_provider_accounts_dir(provider);
    load_accounts_in_dir(&dir)
}

pub(crate) fn save_accounts_in_dir(dir: &Path, accounts: &[AccountProfile]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("Failed to create accounts directory: {}", e))?;

    let index_file = dir.join("accounts.json");
    let serialized = serde_json::to_string_pretty(accounts).map_err(|e| e.to_string())?;

    let mut temp = tempfile::Builder::new()
        .prefix(".accounts.")
        .suffix(".tmp")
        .tempfile_in(dir)
        .map_err(|e| e.to_string())?;

    temp.write_all(serialized.as_bytes()).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(&index_file).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn save_accounts(provider: &str, accounts: &[AccountProfile]) -> Result<(), String> {
    let dir = get_provider_accounts_dir(provider);
    save_accounts_in_dir(&dir, accounts)
}

pub fn save_account_data(provider: &str, account_id: &str, filename: &str, data: &[u8]) -> Result<(), String> {
    let acct_dir = get_account_dir(provider, account_id);
    fs::create_dir_all(&acct_dir).map_err(|e| format!("Failed to create account folder: {}", e))?;

    let file_path = acct_dir.join(filename);
    let mut temp = tempfile::Builder::new()
        .prefix(".data.")
        .suffix(".tmp")
        .tempfile_in(&acct_dir)
        .map_err(|e| e.to_string())?;

    temp.write_all(data).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(&file_path).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn read_account_data(provider: &str, account_id: &str, filename: &str) -> Result<Vec<u8>, String> {
    let file_path = get_account_dir(provider, account_id).join(filename);
    fs::read(&file_path).map_err(|e| format!("Failed to read account data file '{}': {}", filename, e))
}

pub(crate) fn delete_account_files_in_dir(dir: &Path, account_id: &str) -> Result<(), String> {
    let acct_dir = dir.join(account_id);
    match fs::remove_dir_all(&acct_dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "Failed to remove account folder '{}': {}",
            acct_dir.display(),
            e
        )),
    }
}

pub fn delete_account_files(provider: &str, account_id: &str) -> Result<(), String> {
    let dir = get_provider_accounts_dir(provider);
    delete_account_files_in_dir(&dir, account_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_dir_resolution() {
        let dir = get_provider_accounts_dir("antigravity");
        assert!(dir.to_string_lossy().contains("antigravity"));
        let acct_dir = get_account_dir("chatgpt", "acc_123");
        assert!(acct_dir.to_string_lossy().contains("acc_123"));
    }
}

