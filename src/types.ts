export type RunStatus =
  | "stopped"
  | "running_proxied"
  | "running_direct"
  | "running_unproxied"
  | "restart_needed";

export type CheckStatus = "pass" | "warn" | "fail" | "skipped";

export interface ProxyConfig {
  scheme: string;
  host: string;
  port: number;
  no_proxy: string;
}

export interface TargetOverride {
  exe_path: string;
  proxy_enabled: boolean;
}

export interface TargetAppSpec {
  id: string;
  name: string;
  name_en: string;
  builtin: boolean;
  exe_path: string;
  proxy_enabled: boolean;
  probe_urls: string[];
  child_process_patterns: string[];
  sidecar_cmdline_regex: string;
  warn_unsaved_on_kill: boolean;
}

export interface AppRuntimeState {
  app_id: string;
  status: RunStatus;
  pids: number[];
  exe_path: string;
  matched_proxy: string;
  cmdline_preview: string;
}

export interface DiagItem {
  title: string;
  status: CheckStatus;
  summary: string;
  details: string[];
}

export interface DiagReport {
  app_id: string;
  app_name: string;
  proxy_url: string;
  timestamp: string;
  items: DiagItem[];
  raw_summary: string;
}

export interface AppSettings {
  language: string;
  theme: "system" | "light" | "dark";
  autostart: boolean;
  proxy: ProxyConfig;
  targets: Record<string, TargetOverride>;
  custom_apps: TargetAppSpec[];
}
