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

export interface QuotaWindow {
  name: string;
  used_percent?: number;
  remaining_percent?: number;
  resets_at?: string;
  reset_in_seconds?: number;
  detail?: string;
}

export interface AccountQuota {
  provider: string;
  account_id: string;
  email?: string;
  plan_type?: string;
  fetched_at: string;
  windows: QuotaWindow[];
  extra_info: Record<string, string>;
}

export interface AccountProfile {
  id: string;
  provider: string;
  name: string;
  email?: string;
  account_id?: string;
  plan_type?: string;
  created_at: string;
  last_used_at?: string;
  is_active: boolean;
  quota?: AccountQuota;
}

export interface AccountSwitchResult {
  success: boolean;
  restarted_app: boolean;
  message: string;
}

