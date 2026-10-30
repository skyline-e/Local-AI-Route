pub mod commands;
pub mod core;
pub mod platforms;

use commands::AppState;
use core::controller::{ensure_stopped_before_relaunch, inspect_app_state, is_executable_present, launch_target};
use core::models::{RunStatus, TargetOverride};
use core::presets::find_target_spec;
use core::storage::{load_settings, save_settings};
use std::path::Path;
use std::sync::Mutex;
use sysinfo::{ProcessesToUpdate, System};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let mut launch_target_id: Option<String> = None;
    for i in 0..args.len() {
        if args[i] == "--launch" && i + 1 < args.len() {
            launch_target_id = Some(args[i + 1].clone());
            break;
        }
    }

    let mut initial_settings = load_settings();

    // If --launch <app_id> was requested via CLI / desktop shortcut
    if let Some(target_id) = launch_target_id {
        if let Some(mut target) = find_target_spec(&initial_settings, &target_id) {
            if target.builtin && (target.exe_path.is_empty() || !is_executable_present(Path::new(&target.exe_path))) {
                if let Some(detected) = crate::platforms::discover_target_executable(&target.id) {
                    target.exe_path = detected.clone();
                    let entry = initial_settings
                        .targets
                        .entry(target.id.clone())
                        .or_insert_with(TargetOverride::default);
                    entry.exe_path = detected;
                    let _ = save_settings(&initial_settings);
                }
            }
            if target.exe_path.is_empty() {
                crate::platforms::show_native_message_box(
                    "Local-AI-Route 启动失败",
                    &format!("未检测到应用「{}」的可执行文件路径。\n\n请先打开 Local-AI-Route 主窗口完成路径配置。", target.name),
                );
            } else {
                if !target.warn_unsaved_on_kill {
                    let mut sys = System::new();
                    sys.refresh_processes(ProcessesToUpdate::All, true);
                    let runtime = inspect_app_state(&sys, &target, &initial_settings.proxy);
                    if matches!(runtime.status, RunStatus::RestartNeeded | RunStatus::RunningUnproxied) {
                        ensure_stopped_before_relaunch(&runtime.pids, true);
                    }
                }
                if let Err(e) = launch_target(&target, &initial_settings.proxy) {
                    crate::platforms::show_native_message_box(
                        "Local-AI-Route 启动失败",
                        &format!("启动目标应用「{}」失败: {}\n\n请检查路径是否正确或文件是否被占用/拦截。", target.name, e),
                    );
                }
            }
        } else {
            crate::platforms::show_native_message_box(
                "Local-AI-Route 启动失败",
                &format!("未在配置中找到目标应用 ID: {}\n\n请打开 Local-AI-Route 主窗口检查应用列表。", target_id),
            );
        }
        return;
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .manage(AppState {
            settings: Mutex::new(initial_settings),
            sys: Mutex::new(System::new_all()),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::get_effective_targets,
            commands::save_settings,
            commands::scan_proxy_ports,
            commands::auto_detect_target,
            commands::get_app_statuses,
            commands::launch_app,
            commands::stop_app,
            commands::run_diagnostics,
            commands::create_shortcut
        ])
        .setup(|app| {
            // Build Tray Menu
            let show_i = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
            let hide_i = MenuItem::with_id(app, "hide", "隐藏到后台", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "彻底退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &hide_i, &quit_i])?;

            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Local-AI-Route")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "hide" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Prevent window from closing, hide it to tray instead
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running local-ai-route");
}
