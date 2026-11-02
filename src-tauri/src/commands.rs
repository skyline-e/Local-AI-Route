use crate::core::controller::{
    ensure_stopped_before_relaunch, inspect_app_state, launch_target, refresh_processes_for_inspection,
    should_heal_builtin_path, stop_target, sync_gemini_profile_proxy,
};
use crate::core::diagnostics::run_diagnostics as run_diag_internal;
use crate::core::models::{AppRuntimeState, AppSettings, DiagReport, RunStatus, TargetAppSpec, TargetOverride};
use crate::core::presets::{find_target_spec, get_effective_targets as get_effective_targets_internal};
use crate::core::proxy::scan_local_proxy_ports as scan_ports_internal;
use crate::core::storage::save_settings as save_settings_internal;
use crate::platforms::{create_desktop_shortcut, discover_target_executable};
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
        if should_heal_builtin_path(&entry.exe_path) {
            if let Some(detected) = discover_target_executable(&preset.id) {
                if entry.exe_path != detected {
                    entry.exe_path = detected;
                    updated = true;
                }
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

fn resolve_and_heal_target(settings: &mut AppSettings, app_id: &str) -> Result<TargetAppSpec, String> {
    let mut target = find_target_spec(settings, app_id)
        .ok_or_else(|| format!("Target app '{}' not found", app_id))?;

    if target.builtin && should_heal_builtin_path(&target.exe_path) {
        if let Some(detected) = discover_target_executable(&target.id) {
            if target.exe_path != detected {
                target.exe_path = detected.clone();
                let entry = settings.targets.entry(target.id.clone()).or_insert_with(TargetOverride::default);
                entry.exe_path = detected;
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
