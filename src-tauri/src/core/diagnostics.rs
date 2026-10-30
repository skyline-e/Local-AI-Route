use super::controller::{collect_process_tree, extract_proxy_arg, inspect_app_state};
use super::models::{CheckStatus, DiagItem, DiagReport, ProxyConfig, TargetAppSpec};
use std::time::Duration;
use sysinfo::{Pid, ProcessesToUpdate, System};

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

    // Refresh system processes for local stages
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

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
        let detected = extract_proxy_arg(
            &runtime.cmdline_preview.split_whitespace().map(|s| s.to_string()).collect::<Vec<_>>(),
        );

        let (status, summary) = if !spec.proxy_enabled {
            if detected.is_none() {
                (CheckStatus::Pass, "主进程未携带 --proxy-server，符合直连运行预期".into())
            } else {
                (CheckStatus::Fail, "目标处于直连模式，但检测到仍残留代理参数，需重启生效".into())
            }
        } else {
            match detected {
                Some(ref val) if val.to_lowercase().contains(&expected_authority) => {
                    (CheckStatus::Pass, format!("主进程已携带精确代理参数: --proxy-server={}", val))
                }
                Some(ref val) => {
                    (CheckStatus::Warn, format!("主进程代理参数 ({}) 与当前配置不一致，需重启更新", val))
                }
                None => (CheckStatus::Fail, "主进程未检测到 --proxy-server 参数，未成功走代理".into()),
            }
        };

        items.push(DiagItem {
            title: "主进程启动参数核验 (CLI Args Check)".into(),
            status,
            summary,
            details: vec![format!("主进程命令行: {}", redact_path(&runtime.cmdline_preview))],
        });
    }

    // Stage 3: Subprocess Env Check (Antigravity specialized check)
    if runtime.pids.is_empty() {
        items.push(DiagItem {
            title: "关键子进程环境变量核验 (Subprocess Env Check)".into(),
            status: CheckStatus::Skipped,
            summary: "应用未运行，跳过子进程检测".into(),
            details: vec![],
        });
    } else {
        let tree_pids = collect_process_tree(&sys, &runtime.pids);
        let mut matched_children = Vec::new();

        for pid in &tree_pids {
            if let Some(proc) = sys.process(Pid::from_u32(*pid)) {
                let name = proc.name().to_string_lossy().to_lowercase();
                if spec.child_process_patterns.iter().any(|pat| name.contains(&pat.to_lowercase())) {
                    let envs = proc.environ();
                    let https_proxy = envs.iter().find(|e| e.to_string_lossy().starts_with("HTTPS_PROXY="));
                    matched_children.push((name, *pid, https_proxy.map(|s| s.to_string_lossy().to_string())));
                }
            }
        }

        if matched_children.is_empty() {
            items.push(DiagItem {
                title: "关键子进程环境变量核验 (Subprocess Env Check)".into(),
                status: CheckStatus::Pass,
                summary: "未发现独立 language_server 子进程（或当前应用无需独立服务进程）".into(),
                details: vec![],
            });
        } else {
            let mut all_ok = true;
            let mut details = Vec::new();
            for (name, pid, env) in matched_children {
                match env {
                    Some(e) => details.push(format!("✔ 子进程 {} (PID: {}) 包含环境变量: {}", name, pid, e)),
                    None => {
                        all_ok = false;
                        details.push(format!("✘ 子进程 {} (PID: {}) 缺少 HTTPS_PROXY 环境变量", name, pid));
                    }
                }
            }
            items.push(DiagItem {
                title: "关键子进程环境变量核验 (Subprocess Env Check)".into(),
                status: if all_ok { CheckStatus::Pass } else { CheckStatus::Warn },
                summary: if all_ok { "所有关键子进程均正常继承代理环境变量".into() } else { "部分子进程缺少代理环境变量".into() },
                details,
            });
        }
    }

    // Stage 4: PID-tree TCP Snapshot
    let tree_pids = collect_process_tree(&sys, &runtime.pids);
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
            (
                CheckStatus::Pass,
                format!("已快照目标应用进程树（{} 个进程），暂未捕获到活跃的 ESTABLISHED/代理 TCP 连接（应用可能尚未发起网络流量）", tree_pids.len()),
                vec![format!("监控进程 PID 集合: {:?}", tree_pids)],
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
