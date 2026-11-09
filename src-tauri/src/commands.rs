use crate::core::controller::{
    ensure_stopped_before_relaunch, inspect_app_state, is_executable_present, launch_target,
    refresh_processes_for_inspection, should_heal_builtin_path, stop_target, sync_gemini_profile_proxy,
};
use crate::core::diagnostics::run_diagnostics as run_diag_internal;
use crate::core::models::{AppRuntimeState, AppSettings, DiagReport, RunStatus, TargetAppSpec, TargetOverride};
use crate::core::presets::{find_target_spec, get_effective_targets as get_effective_targets_internal};
use crate::core::proxy::scan_local_proxy_ports as scan_ports_internal;
use crate::core::storage::save_settings as save_settings_internal;
use crate::platforms::{create_desktop_shortcut, discover_target_executable, resolve_executable_for_launch};
use std::path::Path;
use std::sync::Mutex;
use sysinfo::System;
use tauri::State;

use tauri_plugin_autostart::ManagerExt;

pub struct AppState {
    pub settings: Mutex<AppSettings>,
    pub sys: Mutex<System>,
}

fn safe_lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle, state: State<'_, AppState>) -> AppSettings {
    let mut settings = safe_lock(&state.settings);

    // Sync autostart from system registry / launch agent
    if let Ok(enabled) = app.autolaunch().is_enabled() {
        settings.autostart = enabled;
    }

    // Auto-detect built-in targets if paths are currently empty, stale, or inside versioned MSIX/Squirrel folders
    let mut updated = false;
    for preset in crate::core::presets::builtin_presets() {
        let entry = settings.targets.entry(preset.id.clone()).or_insert_with(TargetOverride::default);
        if should_heal_builtin_path(&preset.id, &entry.exe_path) {
            let candidate = discover_target_executable(&preset.id).or_else(|| {
                if !entry.exe_path.is_empty() {
                    let resolved = resolve_executable_for_launch(&entry.exe_path);
                    if resolved != entry.exe_path && is_executable_present(Path::new(&resolved)) {
                        return Some(resolved);
                    }
                }
                None
            });
            if let Some(detected) = candidate {
                if entry.exe_path != detected {
                    entry.exe_path = detected;
                    updated = true;
                }
            }
        }
    }
    for custom in &mut settings.custom_apps {
        if !custom.exe_path.is_empty() && should_heal_builtin_path(&custom.id, &custom.exe_path) {
            let resolved = resolve_executable_for_launch(&custom.exe_path);
            if resolved != custom.exe_path && is_executable_present(Path::new(&resolved)) {
                custom.exe_path = resolved;
                updated = true;
            }
        }
    }
    if updated {
        let _ = save_settings_internal(&settings);
    }

    settings.clone()
}

#[tauri::command]
pub fn get_effective_targets(state: State<'_, AppState>) -> Vec<TargetAppSpec> {
    let settings = safe_lock(&state.settings);
    get_effective_targets_internal(&settings)
}

#[tauri::command]
pub fn save_settings(app: tauri::AppHandle, new_settings: AppSettings, state: State<'_, AppState>) -> Result<(), String> {
    save_settings_internal(&new_settings)?;
    let autolaunch = app.autolaunch();
    if new_settings.autostart {
        let _ = autolaunch.enable();
    } else {
        let _ = autolaunch.disable();
    }
    if let Some(gemini_target) = find_target_spec(&new_settings, "gemini") {
        sync_gemini_profile_proxy(&new_settings.proxy, gemini_target.proxy_enabled);
    }
    crate::update_tray_menu(&app, &new_settings.language);
    let mut current = safe_lock(&state.settings);
    *current = new_settings;
    Ok(())
}

#[tauri::command]
pub async fn scan_proxy_ports() -> Vec<u16> {
    scan_ports_internal().await
}

#[tauri::command]
pub fn auto_detect_target(app_id: String) -> Option<String> {
    discover_target_executable(&app_id)
}

#[tauri::command]
pub fn get_app_statuses(state: State<'_, AppState>) -> Vec<AppRuntimeState> {
    let settings = safe_lock(&state.settings);
    let mut sys = safe_lock(&state.sys);
    refresh_processes_for_inspection(&mut sys, false);

    let targets = get_effective_targets_internal(&settings);
    let mut statuses = Vec::new();

    for target in targets {
        statuses.push(inspect_app_state(&sys, &target, &settings.proxy));
    }
    statuses
}

pub(crate) fn resolve_and_heal_target(settings: &mut AppSettings, app_id: &str) -> Result<TargetAppSpec, String> {
    let mut target = find_target_spec(settings, app_id)
        .ok_or_else(|| format!("Target app '{}' not found", app_id))?;

    if should_heal_builtin_path(&target.id, &target.exe_path) {
        let candidate = if target.builtin {
            discover_target_executable(&target.id)
        } else {
            None
        }
        .or_else(|| {
            if !target.exe_path.is_empty() {
                let resolved = resolve_executable_for_launch(&target.exe_path);
                if resolved != target.exe_path && is_executable_present(Path::new(&resolved)) {
                    return Some(resolved);
                }
            }
            None
        });

        if let Some(detected) = candidate {
            if target.exe_path != detected {
                target.exe_path = detected.clone();
                if target.builtin {
                    let entry = settings
                        .targets
                        .entry(target.id.clone())
                        .or_insert_with(TargetOverride::default);
                    entry.exe_path = detected;
                } else if let Some(custom) = settings.custom_apps.iter_mut().find(|c| c.id == target.id) {
                    custom.exe_path = detected;
                }
                let _ = save_settings_internal(settings);
            }
        }
    }

    Ok(target)
}

#[tauri::command]
pub async fn launch_app(
    app_id: String,
    force_restart: Option<bool>,
    state: State<'_, AppState>,
) -> Result<u32, String> {
    let (target, proxy, pids_to_restart) = {
        let mut settings = safe_lock(&state.settings);
        let target = resolve_and_heal_target(&mut settings, &app_id)?;

        let mut sys = safe_lock(&state.sys);
        refresh_processes_for_inspection(&mut sys, false);
        let runtime = inspect_app_state(&sys, &target, &settings.proxy);
        let pids = if matches!(runtime.status, RunStatus::RestartNeeded | RunStatus::RunningUnproxied) {
            runtime.pids
        } else {
            vec![]
        };

        (target, settings.proxy.clone(), pids)
    };

    if !pids_to_restart.is_empty() {
        let allow_force_kill = force_restart.unwrap_or(!target.warn_unsaved_on_kill);
        let stopped = tokio::task::spawn_blocking(move || {
            ensure_stopped_before_relaunch(&pids_to_restart, allow_force_kill)
        })
        .await
        .map_err(|e| format!("Restart task error: {}", e))?;

        if !stopped {
            return Err("已向应用发送优雅关闭信号，请先在应用内确认保存文件并退出后再点击启动。".into());
        }
    }

    let child_pid = launch_target(&target, &proxy)?;
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    Ok(child_pid)
}

#[tauri::command]
pub fn stop_app(app_id: String, force: bool, state: State<'_, AppState>) -> Result<(), String> {
    let settings = safe_lock(&state.settings);
    let mut sys = safe_lock(&state.sys);
    refresh_processes_for_inspection(&mut sys, false);

    let target = find_target_spec(&settings, &app_id)
        .ok_or_else(|| format!("Target app '{}' not found", app_id))?;

    let runtime = inspect_app_state(&sys, &target, &settings.proxy);
    stop_target(&sys, &runtime.pids, force)
}

#[tauri::command]
pub async fn run_diagnostics(app_id: String, state: State<'_, AppState>) -> Result<DiagReport, String> {
    let (target, proxy) = {
        let mut settings = safe_lock(&state.settings);
        let target = resolve_and_heal_target(&mut settings, &app_id)?;
        (target, settings.proxy.clone())
    };

    Ok(run_diag_internal(&target, &proxy).await)
}

#[tauri::command]
pub fn create_shortcut(app_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let mut settings = safe_lock(&state.settings);
    let target = resolve_and_heal_target(&mut settings, &app_id)?;

    create_desktop_shortcut(&target.id, &target.name, &target.exe_path)
}

use crate::accounts::models::{AccountProfile, AccountQuota, AccountSwitchResult};

#[tauri::command]
pub fn get_accounts(app_id: String) -> Result<Vec<AccountProfile>, String> {
    crate::accounts::list_accounts(&app_id)
}

#[tauri::command]
pub async fn import_current_account(
    app_id: String,
    custom_name: Option<String>,
    state: State<'_, AppState>,
) -> Result<AccountProfile, String> {
    let proxy = {
        let settings = safe_lock(&state.settings);
        settings.proxy.clone()
    };
    crate::accounts::import_current_account(&app_id, custom_name, Some(&proxy)).await
}

#[tauri::command]
pub async fn add_manual_account(
    app_id: String,
    name: String,
    raw_content: String,
    state: State<'_, AppState>,
) -> Result<AccountProfile, String> {
    let proxy = {
        let settings = safe_lock(&state.settings);
        settings.proxy.clone()
    };
    crate::accounts::add_account_manual(&app_id, name, raw_content, Some(&proxy)).await
}

#[tauri::command]
pub async fn switch_account(
    app_id: String,
    account_id: String,
    state: State<'_, AppState>,
) -> Result<AccountSwitchResult, String> {
    let (target_spec, proxy, pids) = {
        let mut settings = safe_lock(&state.settings);
        let target = resolve_and_heal_target(&mut settings, &app_id).ok();

        let mut sys = safe_lock(&state.sys);
        refresh_processes_for_inspection(&mut sys, false);
        let pids = if let Some(ref t) = target {
            let runtime = inspect_app_state(&sys, t, &settings.proxy);
            if matches!(
                runtime.status,
                RunStatus::RunningProxied
                    | RunStatus::RunningDirect
                    | RunStatus::RunningUnproxied
                    | RunStatus::RestartNeeded
            ) {
                runtime.pids
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        (target, settings.proxy.clone(), pids)
    };

    crate::accounts::switch_account(
        &app_id,
        &account_id,
        pids,
        target_spec.as_ref(),
        Some(&proxy),
    )
    .await
}

#[tauri::command]
pub fn delete_account(app_id: String, account_id: String) -> Result<(), String> {
    crate::accounts::delete_account(&app_id, &account_id)
}

#[tauri::command]
pub async fn refresh_quota(
    app_id: String,
    account_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<AccountQuota, String> {
    let proxy = {
        let settings = safe_lock(&state.settings);
        settings.proxy.clone()
    };
    crate::accounts::refresh_quota(&app_id, account_id, Some(&proxy)).await
}

#[tauri::command]
pub fn prepare_new_claude_login() -> Result<String, String> {
    crate::accounts::prepare_new_claude_login()
}

#[tauri::command]
pub fn prepare_new_factory_login() -> Result<String, String> {
    crate::accounts::factory::prepare_new_factory_login()
}
