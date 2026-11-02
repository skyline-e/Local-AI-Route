use super::controller::{collect_process_tree, extract_proxy_arg, inspect_app_state, refresh_processes_for_inspection};
use super::models::{CheckStatus, DiagItem, DiagReport, ProxyConfig, TargetAppSpec};
use std::time::Duration;
use sysinfo::{Pid, System};

pub fn redact_path(input: &str) -> String {
    let mut out = input.to_string();
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy().to_string();
        let home_slash = home_str.replace('\\', "/");
        let pattern = format!("(?i)({}|{})", regex::escape(&home_str), regex::escape(&home_slash));
        if let Ok(re) = regex::Regex::new(&pattern) {
            out = re.replace_all(&out, "%USERPROFILE%").to_string();
        } else {
            out = out.replace(&home_str, "%USERPROFILE%");
            out = out.replace(&home_slash, "%USERPROFILE%");
        }
    }
    out
}

pub async fn run_diagnostics(spec: &TargetAppSpec, proxy: &ProxyConfig) -> DiagReport {
    let mut items = Vec::new();
    let proxy_url = proxy.url();

    // Stage 1: Tunnel Probe
    if !spec.proxy_enabled {
        items.push(DiagItem {
            title: "代理隧道探测 (Tunnel Probe)".into(),
            status: CheckStatus::Skipped,
            summary: "当前应用设置为直连模式，已跳过代理连通性探测。".into(),
            details: vec![],
        });
    } else {
        let mut probe_details = Vec::new();
        let mut any_ok = false;
        let mut summary_msg = String::new();

        let client = match reqwest::Proxy::all(&proxy_url) {
            Ok(p) => reqwest::Client::builder()
                .proxy(p)
                .timeout(Duration::from_secs(6))
                .build()
                .ok(),
            Err(e) => {
                summary_msg = format!("代理配置语法错误: {}", e);
                None
            }
        };

        if let Some(client) = client {
            let urls = if spec.probe_urls.is_empty() {
                vec!["https://www.gstatic.com/generate_204".to_string()]
            } else {
                spec.probe_urls.clone()
            };

            for url in &urls {
                match client.get(url).send().await {
                    Ok(resp) => {
                        let status = resp.status().as_u16();
                        any_ok = true;
                        let note = if status == 403 {
                            " (403 表示代理及 TLS 隧道握手已成功建立，仅因无凭据被目标反爬拦截)"
                        } else {
                            ""
                        };
                        probe_details.push(format!("✔ 成功访问 {} -> HTTP {}{}", url, status, note));
                        if summary_msg.is_empty() {
                            summary_msg = format!("代理隧道连通正常 (HTTP {})", status);
                        }
                    }
                    Err(e) => {
                        probe_details.push(format!("✘ 访问 {} 失败: {}", url, e));
                        if summary_msg.is_empty() {
                            summary_msg = format!("代理上游握手失败: {}", e);
                        }
                    }
                }
            }
        }

        items.push(DiagItem {
            title: "代理隧道探测 (Tunnel Probe)".into(),
            status: if any_ok { CheckStatus::Pass } else { CheckStatus::Fail },
            summary: summary_msg,
            details: probe_details,
        });
    }

    // Refresh system processes for local stages (including cmdline and environment variables)
    let mut sys = System::new();
    refresh_processes_for_inspection(&mut sys, true);

    let runtime = inspect_app_state(&sys, spec, proxy);

    // Stage 2: CLI Args Check
    if runtime.pids.is_empty() {
        items.push(DiagItem {
            title: "主进程启动参数核验 (CLI Args Check)".into(),
            status: CheckStatus::Warn,
            summary: "目标应用当前未运行，无法核验实时命令行参数。".into(),
            details: vec!["请在卡片中点击「启动」后再执行完整诊断。".into()],
        });
    } else {
        let expected_authority = proxy.authority().to_lowercase();
        let detected = if !runtime.matched_proxy.is_empty() {
            Some(runtime.matched_proxy.clone())
        } else {
            extract_proxy_arg(
                &runtime.cmdline_preview.split_whitespace().map(|s| s.to_string()).collect::<Vec<_>>(),
            )
        };

        let (status, summary) = if !spec.proxy_enabled {
            if detected.is_none() {
                if spec.id == "gemini" {
                    (CheckStatus::Pass, "Gemini Profile 未注入代理配置，符合直连运行预期".into())
                } else {
                    (CheckStatus::Pass, "主进程未携带 --proxy-server，符合直连运行预期".into())
                }
            } else {
                (CheckStatus::Fail, "目标处于直连模式，但检测到仍残留代理参数，需重启生效".into())
            }
        } else {
            match detected {
                Some(ref val) if val.to_lowercase().contains(&expected_authority) => {
                    if spec.id == "gemini" {
                        (CheckStatus::Pass, format!("Gemini Profile (Preferences) 已正确注入代理: {}", val))
                    } else {
                        (CheckStatus::Pass, format!("主进程已携带精确代理参数: --proxy-server={}", val))
                    }
                }
                Some(ref val) => {
                    (CheckStatus::Warn, format!("代理参数 ({}) 与当前配置不一致，需重启更新", val))
                }
                None => {
                    if spec.id == "gemini" {
                        (CheckStatus::Fail, "Gemini Profile 未检测到 proxy 配置，未成功走代理".into())
                    } else {
                        (CheckStatus::Fail, "主进程未检测到 --proxy-server 参数，未成功走代理".into())
                    }
                }
            }
        };

        let details = if spec.id == "gemini" {
            vec![
                format!("主进程命令行: {}", redact_path(&runtime.cmdline_preview)),
                "注: Gemini 桌面版内置了命令行网络开关拦截检测，因此系统原生采用 Chromium Profile (Preferences) 注入代理，底层 NetworkService 物理流量已完整托管。".into(),
            ]
        } else {
            vec![format!("主进程命令行: {}", redact_path(&runtime.cmdline_preview))]
        };

        items.push(DiagItem {
            title: "主进程启动参数核验 (CLI Args Check)".into(),
            status,
            summary,
            details,
        });
    }

    // Pre-collect process tree once for both Stage 3 and Stage 4
    let mut tree_pids = if runtime.pids.is_empty() {
        std::collections::HashSet::new()
    } else {
        collect_process_tree(&sys, &runtime.pids)
    };

    // Stage 3: Subprocess Env Check (Antigravity specialized check)
    if runtime.pids.is_empty() {
        items.push(DiagItem {
            title: "关键子进程环境变量核验 (Subprocess Env Check)".into(),
            status: CheckStatus::Skipped,
            summary: "应用未运行，跳过子进程检测".into(),
            details: vec![],
        });
    } else if spec.child_process_patterns.is_empty() {
        items.push(DiagItem {
            title: "关键子进程环境变量核验 (Subprocess Env Check)".into(),
            status: CheckStatus::Pass,
            summary: "当前应用无需独立服务子进程校验，网络请求由主进程及 Chromium 网络栈直接接管".into(),
            details: vec![],
        });
    } else {
        let sidecar_regex = if !spec.sidecar_cmdline_regex.is_empty() {
            regex::Regex::new(&spec.sidecar_cmdline_regex).ok()
        } else {
            None
        };
        let mut matched_children = Vec::new();
        let mut sorted_tree_pids: Vec<u32> = tree_pids.iter().copied().collect();
        sorted_tree_pids.sort_unstable();

        for pid in &sorted_tree_pids {
            if let Some(proc) = sys.process(Pid::from_u32(*pid)) {
                let name = proc.name().to_string_lossy().to_lowercase();
                if spec.child_process_patterns.iter().any(|pat| name.contains(&pat.to_lowercase())) {
                    let cmd_joined = proc
                        .cmd()
                        .iter()
                        .map(|s| s.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if let Some(ref re) = sidecar_regex {
                        if !cmd_joined.is_empty() && !re.is_match(&cmd_joined) {
                            continue;
                        }
                    }
                    let envs = proc.environ();
                    let https_proxy = envs
                        .iter()
                        .find(|e| e.to_string_lossy().to_ascii_uppercase().starts_with("HTTPS_PROXY="));
                    matched_children.push((name, *pid, https_proxy.map(|s| s.to_string_lossy().to_string())));
                }
            }
        }

        // Fallback: if sidecar process detached from parent tree, match globally using sidecar_cmdline_regex
        if matched_children.is_empty() {
            if let Some(ref re) = sidecar_regex {
                for (pid, proc) in sys.processes() {
                    let name = proc.name().to_string_lossy().to_lowercase();
                    if spec.child_process_patterns.iter().any(|pat| name.contains(&pat.to_lowercase())) {
                        let cmd_joined = proc
                            .cmd()
                            .iter()
                            .map(|s| s.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(" ");
                        if re.is_match(&cmd_joined) {
                            let pid_u32 = pid.as_u32();
                            tree_pids.insert(pid_u32);
                            let envs = proc.environ();
                            let https_proxy = envs
                                .iter()
                                .find(|e| e.to_string_lossy().to_ascii_uppercase().starts_with("HTTPS_PROXY="));
                            matched_children.push((name, pid_u32, https_proxy.map(|s| s.to_string_lossy().to_string())));
                        }
                    }
                }
                matched_children.sort_by_key(|(_, pid, _)| *pid);
            }
        }

        if matched_children.is_empty() {
            items.push(DiagItem {
                title: "关键子进程环境变量核验 (Subprocess Env Check)".into(),
                status: CheckStatus::Pass,
                summary: "未发现独立 language_server 子进程（或当前应用无需独立服务进程）".into(),
                details: vec![],
            });
        } else if !spec.proxy_enabled {
            let mut all_ok = true;
            let mut details = Vec::new();
            for (name, pid, env) in matched_children {
                match env {
                    None => details.push(format!("✔ 子进程 {} (PID: {}) 未携带代理环境变量，符合直连预期", name, pid)),
                    Some(e) => {
                        all_ok = false;
                        details.push(format!("✘ 子进程 {} (PID: {}) 仍残留代理环境变量: {}", name, pid, e));
                    }
                }
            }
            items.push(DiagItem {
                title: "关键子进程环境变量核验 (Subprocess Env Check)".into(),
                status: if all_ok { CheckStatus::Pass } else { CheckStatus::Warn },
                summary: if all_ok {
                    "直连模式下所有关键子进程均未携带代理环境变量".into()
                } else {
                    "部分子进程仍残留代理环境变量，建议重启应用".into()
                },
                details,
            });
        } else {
            let expected_authority = proxy.authority().to_lowercase();
            let mut all_ok = true;
            let mut details = Vec::new();
            for (name, pid, env) in matched_children {
                match env {
                    Some(e) if e.to_lowercase().contains(&expected_authority) => {
                        details.push(format!("✔ 子进程 {} (PID: {}) 包含匹配环境变量: {}", name, pid, e));
                    }
                    Some(e) => {
                        all_ok = false;
                        details.push(format!("✘ 子进程 {} (PID: {}) 环境变量与当前代理配置不一致: {}", name, pid, e));
                    }
                    None => {
                        all_ok = false;
                        details.push(format!("✘ 子进程 {} (PID: {}) 缺少 HTTPS_PROXY 环境变量", name, pid));
                    }
                }
            }
            items.push(DiagItem {
                title: "关键子进程环境变量核验 (Subprocess Env Check)".into(),
                status: if all_ok { CheckStatus::Pass } else { CheckStatus::Warn },
                summary: if all_ok {
                    "所有关键子进程均正常继承当前代理环境变量".into()
                } else {
                    "部分子进程缺少代理环境变量或与当前配置不一致，建议重启应用".into()
                },
                details,
            });
        }
    }

    // Stage 4: PID-tree TCP Snapshot
    if runtime.pids.is_empty() {
        items.push(DiagItem {
            title: "进程树活跃 TCP 快照 (PID-Tree TCP Snapshot)".into(),
            status: CheckStatus::Skipped,
            summary: "目标应用当前未运行，无活跃进程树连接。".into(),
            details: vec![],
        });
    } else {
        let conn_records = crate::platforms::get_process_tcp_connections(&tree_pids, proxy.port);
        let (status, summary, details) = if !conn_records.is_empty() {
            let proxy_matches = conn_records.iter().filter(|r| r.contains("[指向代理端口]")).count();
            let summary_text = if proxy_matches > 0 {
                format!("✔ 检测到目标进程树共 {} 条网络连接（其中 {} 条精准指向本地代理端口 :{}）", conn_records.len(), proxy_matches, proxy.port)
            } else {
                format!("检测到目标进程树共 {} 条活跃网络连接（暂无直接指向代理端口 :{} 的连接，应用可能已建立保活或处于空闲状态）", conn_records.len(), proxy.port)
            };
            (CheckStatus::Pass, summary_text, conn_records)
        } else {
            let mut sorted_pids: Vec<u32> = tree_pids.iter().copied().collect();
            sorted_pids.sort_unstable();
            (
                CheckStatus::Pass,
                format!("已快照目标应用进程树（{} 个进程），暂未捕获到活跃的 ESTABLISHED/代理 TCP 连接（应用可能尚未发起网络流量）", sorted_pids.len()),
                vec![format!("监控进程 PID 集合: {:?}", sorted_pids)],
            )
        };

        items.push(DiagItem {
            title: "进程树活跃 TCP 快照 (PID-Tree TCP Snapshot)".into(),
            status,
            summary,
            details,
        });
    }

    let raw_summary = items
        .iter()
        .map(|it| format!("[{:?}] {}: {}", it.status, it.title, it.summary))
        .collect::<Vec<_>>()
        .join("\n");

    DiagReport {
        app_id: spec.id.clone(),
        app_name: spec.name.clone(),
        proxy_url,
        timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        items,
        raw_summary,
    }
}
