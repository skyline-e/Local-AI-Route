use crate::core::controller::{inspect_app_state, launch_target, stop_target};
use crate::core::diagnostics::run_diagnostics as run_diag_internal;
use crate::core::models::{AppRuntimeState, AppSettings, DiagReport, TargetAppSpec, TargetOverride};
use crate::core::presets::{find_target_spec, get_effective_targets as get_effective_targets_internal};
use crate::core::proxy::scan_local_proxy_ports as scan_ports_internal;
use crate::core::storage::save_settings as save_settings_internal;
use crate::platforms::{create_desktop_shortcut, discover_target_executable};
use std::sync::Mutex;
use sysinfo::{ProcessesToUpdate, System};
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

    // Auto-detect built-in targets if paths are currently empty
    for preset in crate::core::presets::builtin_presets() {
        let entry = settings.targets.entry(preset.id.clone()).or_insert_with(TargetOverride::default);
        if entry.exe_path.is_empty() {
            if let Some(detected) = discover_target_executable(&preset.id) {
                entry.exe_path = detected;
            }
        }
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
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let targets = get_effective_targets_internal(&settings);
    let mut statuses = Vec::new();

    for target in targets {
        statuses.push(inspect_app_state(&sys, &target, &settings.proxy));
    }
    statuses
}

#[tauri::command]
pub fn launch_app(app_id: String, state: State<'_, AppState>) -> Result<u32, String> {
    let settings = safe_lock(&state.settings);
    let target = find_target_spec(&settings, &app_id)
        .ok_or_else(|| format!("Target app '{}' not found", app_id))?;

    launch_target(&target, &settings.proxy)
}

#[tauri::command]
pub fn stop_app(app_id: String, force: bool, state: State<'_, AppState>) -> Result<(), String> {
    let settings = safe_lock(&state.settings);
    let mut sys = safe_lock(&state.sys);
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let target = find_target_spec(&settings, &app_id)
        .ok_or_else(|| format!("Target app '{}' not found", app_id))?;

    let runtime = inspect_app_state(&sys, &target, &settings.proxy);
    stop_target(&runtime.pids, force)
}

#[tauri::command]
pub async fn run_diagnostics(app_id: String, state: State<'_, AppState>) -> Result<DiagReport, String> {
    let (target, proxy) = {
        let settings = safe_lock(&state.settings);
        let target = find_target_spec(&settings, &app_id)
            .ok_or_else(|| format!("Target app '{}' not found", app_id))?;
        (target, settings.proxy.clone())
    };

    Ok(run_diag_internal(&target, &proxy).await)
}

#[tauri::command]
pub fn create_shortcut(app_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let settings = safe_lock(&state.settings);
    let target = find_target_spec(&settings, &app_id)
        .ok_or_else(|| format!("Target app '{}' not found", app_id))?;

    create_desktop_shortcut(&target.id, &target.name, &target.exe_path)
}
