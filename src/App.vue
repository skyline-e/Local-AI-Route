<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  AppSettings,
  AppRuntimeState,
  DiagReport,
  TargetAppSpec,
  RunStatus,
  AccountProfile,
  AccountQuota,
  AccountSwitchResult,
} from "./types";
import { t, currentLang, detectSystemLanguage } from "./i18n";
import {
  ShieldCheck,
  Zap,
  Globe,
  Play,
  RotateCw,
  Square,
  Activity,
  ExternalLink,
  ChevronDown,
  ChevronUp,
  Plus,
  Check,
  Copy,
  Moon,
  Sun,
  Monitor,
  Search,
  X,
  AlertTriangle,
  AlertCircle,
  Info,
  Trash2,
  Network,
  Sliders,
  Folder,
  Cpu,
  Terminal,
  Users,
  UserCheck,
  RefreshCw,
  Clock,
  ArrowRightLeft,
  Key,
  PlusCircle,
  Calendar,
} from "lucide-vue-next";

const settings = ref<AppSettings>({
  language: "zh-CN",
  theme: "system",
  autostart: false,
  proxy: {
    scheme: "http",
    host: "127.0.0.1",
    port: 7890,
    no_proxy: "localhost,127.0.0.1,::1,[::1],*.local,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16",
  },
  targets: {},
  custom_apps: [],
});

const effectiveTargets = ref<TargetAppSpec[]>([]);
const appStatuses = ref<Record<string, AppRuntimeState>>({});
const launchingIds = ref<Record<string, boolean>>({});
const showNoProxy = ref(false);
const scannedPorts = ref<number[]>([]);
const isScanning = ref(false);
const showPortDropdown = ref(false);
const saveSuccess = ref(false);

// Active navigation item: 'global-proxy' or target app ID
const activeNavId = ref<string>("global-proxy");

const allTargets = computed<TargetAppSpec[]>(() => effectiveTargets.value);

const currentTarget = computed<TargetAppSpec | undefined>(() => {
  return allTargets.value.find((t) => t.id === activeNavId.value);
});

function selectNav(id: string) {
  activeNavId.value = id;
  try {
    localStorage.setItem("local_ai_route_last_tab", id);
  } catch {}
  if (["chatgpt", "claude", "antigravity-app", "antigravity-ide", "factory"].includes(id)) {
    loadAccountsForCurrent();
  }
}

// Toast Notification System
interface Toast {
  id: number;
  message: string;
  type: "success" | "error" | "warn" | "info";
}
const toasts = ref<Toast[]>([]);
let nextToastId = 1;

function showToast(message: string, type: "success" | "error" | "warn" | "info" = "info", duration = 3500) {
  const id = nextToastId++;
  toasts.value.push({ id, message, type });
  setTimeout(() => {
    toasts.value = toasts.value.filter((t) => t.id !== id);
  }, duration);
}

// Confirm Stop Modal
const confirmStopModal = ref<{
  open: boolean;
  target: TargetAppSpec | null;
  relaunchAfterStop: boolean;
}>({
  open: false,
  target: null,
  relaunchAfterStop: false,
});

// Confirm Remove Custom App Modal
const confirmRemoveModal = ref<{
  open: boolean;
  appId: string;
  appName: string;
  isRunning: boolean;
}>({
  open: false,
  appId: "",
  appName: "",
  isRunning: false,
});

// Diagnostics Modal
const activeDiagReport = ref<DiagReport | null>(null);
const isRunningDiag = ref(false);
const diagModalOpen = ref(false);

// Custom App Modal
const addModalOpen = ref(false);
const customAppForm = ref({
  id: "",
  name: "",
  name_en: "",
  exe_path: "",
  probe_url: "",
  warn_unsaved: false,
});

let statusPollTimer: number | null = null;

function applyTheme(theme: "system" | "light" | "dark") {
  settings.value.theme = theme;
  if (theme === "dark") {
    document.documentElement.classList.add("dark");
  } else if (theme === "light") {
    document.documentElement.classList.remove("dark");
  } else {
    if (window.matchMedia("(prefers-color-scheme: dark)").matches) {
      document.documentElement.classList.add("dark");
    } else {
      document.documentElement.classList.remove("dark");
    }
  }
}

async function refreshTargets() {
  try {
    const list = await invoke<TargetAppSpec[]>("get_effective_targets");
    effectiveTargets.value = list;
    if (activeNavId.value !== "global-proxy" && !list.some((t) => t.id === activeNavId.value)) {
      if (list.length > 0) {
        selectNav(list[0].id);
      } else {
        selectNav("global-proxy");
      }
    }
  } catch (err) {
    console.error("Failed to load effective targets:", err);
  }
}

async function loadInitialData() {
  try {
    const res = await invoke<AppSettings>("get_settings");
    settings.value = res;
    if (res.language === "en-US" || res.language === "zh-CN") {
      currentLang.value = res.language;
    } else {
      currentLang.value = detectSystemLanguage();
      settings.value.language = currentLang.value;
    }
    applyTheme(res.theme || "system");
    await refreshTargets();
    await refreshStatuses();

    // Restore last active navigation tab
    let savedTab: string | null = null;
    try {
      savedTab = localStorage.getItem("local_ai_route_last_tab");
    } catch {}
    if (savedTab && (savedTab === "global-proxy" || effectiveTargets.value.some((t) => t.id === savedTab))) {
      activeNavId.value = savedTab;
    } else if (effectiveTargets.value.length > 0) {
      activeNavId.value = effectiveTargets.value[0].id;
    } else {
      activeNavId.value = "global-proxy";
    }
  } catch (err) {
    showToast(`Failed to load settings: ${err}`, "error");
  }
}

async function refreshStatuses() {
  try {
    const list = await invoke<AppRuntimeState[]>("get_app_statuses");
    const map: Record<string, AppRuntimeState> = {};
    for (const item of list) {
      map[item.app_id] = item;
    }
    appStatuses.value = map;
  } catch (err) {
    console.error("Failed to refresh statuses:", err);
  }
}

async function saveCurrentSettings() {
  try {
    settings.value.language = currentLang.value;
    await invoke("save_settings", { newSettings: settings.value });
    saveSuccess.value = true;
    setTimeout(() => {
      saveSuccess.value = false;
    }, 2500);
    await refreshTargets();
    await refreshStatuses();
  } catch (err) {
    showToast(`Save error: ${err}`, "error");
  }
}

async function handleScanPorts() {
  isScanning.value = true;
  try {
    const ports = await invoke<number[]>("scan_proxy_ports");
    scannedPorts.value = ports;
    showPortDropdown.value = true;
  } catch (err) {
    showToast(`Port scan error: ${err}`, "error");
  } finally {
    isScanning.value = false;
  }
}

function selectScannedPort(port: number) {
  settings.value.proxy.port = port;
  showPortDropdown.value = false;
}

async function toggleProxyMode(target: TargetAppSpec) {
  const nextVal = !target.proxy_enabled;
  if (target.builtin) {
    if (!settings.value.targets[target.id]) {
      settings.value.targets[target.id] = { exe_path: target.exe_path, proxy_enabled: nextVal };
    } else {
      settings.value.targets[target.id].proxy_enabled = nextVal;
    }
  } else {
    target.proxy_enabled = nextVal;
    const custom = settings.value.custom_apps.find((a) => a.id === target.id);
    if (custom) {
      custom.proxy_enabled = nextVal;
    }
  }
  await saveCurrentSettings();
}

async function onTargetExeChange(target: TargetAppSpec) {
  if (target.builtin) {
    if (!settings.value.targets[target.id]) {
      settings.value.targets[target.id] = { exe_path: target.exe_path, proxy_enabled: target.proxy_enabled };
    } else {
      settings.value.targets[target.id].exe_path = target.exe_path;
    }
  } else {
    const custom = settings.value.custom_apps.find((a) => a.id === target.id);
    if (custom) {
      custom.exe_path = target.exe_path;
    }
  }
  await saveCurrentSettings();
}

async function handleAutoDetect(appId: string) {
  try {
    const detected = await invoke<string | null>("auto_detect_target", { appId });
    if (detected) {
      if (!settings.value.targets[appId]) {
        settings.value.targets[appId] = { exe_path: detected, proxy_enabled: true };
      } else {
        settings.value.targets[appId].exe_path = detected;
      }
      await saveCurrentSettings();
      showToast(t("toast_auto_detect_success"), "success");
    } else {
      showToast(t("toast_auto_detect_failed"), "warn");
    }
  } catch (err) {
    showToast(`Auto-detect error: ${err}`, "error");
  }
}

function handleLaunch(target: TargetAppSpec) {
  if (launchingIds.value[target.id]) return;
  const status = appStatuses.value[target.id]?.status;
  const needsRestart = status === "restart_needed" || status === "running_unproxied";
  if (needsRestart && target.warn_unsaved_on_kill) {
    confirmStopModal.value = { open: true, target, relaunchAfterStop: true };
  } else {
    executeLaunch(target);
  }
}

async function executeLaunch(target: TargetAppSpec, forceRestart?: boolean) {
  if (launchingIds.value[target.id]) return;
  launchingIds.value[target.id] = true;
  try {
    confirmStopModal.value = { open: false, target: null, relaunchAfterStop: false };
    await invoke("launch_app", { appId: target.id, forceRestart });
    await refreshTargets();
    await refreshStatuses();
  } catch (err) {
    showToast(`${err}`, "warn", 4500);
    await refreshStatuses();
  } finally {
    launchingIds.value[target.id] = false;
  }
}

function handleStop(target: TargetAppSpec) {
  if (target.warn_unsaved_on_kill) {
    confirmStopModal.value = { open: true, target, relaunchAfterStop: false };
  } else {
    executeStop(target, false);
  }
}

async function executeStop(target: TargetAppSpec, force = false) {
  try {
    await invoke("stop_app", { appId: target.id, force });
    await refreshStatuses();
    showToast(force ? t("toast_force_stopped") : t("toast_graceful_stopped"), "info");
  } catch (err) {
    showToast(`Stop failed: ${err}`, "error");
  } finally {
    confirmStopModal.value = { open: false, target: null, relaunchAfterStop: false };
  }
}

async function handleRunDiagnostics(target: TargetAppSpec) {
  isRunningDiag.value = true;
  try {
    const report = await invoke<DiagReport>("run_diagnostics", { appId: target.id });
    activeDiagReport.value = report;
    diagModalOpen.value = true;
  } catch (err) {
    showToast(`Diagnostics error: ${err}`, "error");
  } finally {
    isRunningDiag.value = false;
  }
}

async function handleCreateShortcut(target: TargetAppSpec) {
  try {
    const res = await invoke<string>("create_shortcut", { appId: target.id });
    showToast(`${t("shortcut_created")} (${res})`, "success", 4000);
  } catch (err) {
    showToast(`Shortcut error: ${err}`, "error");
  }
}

function handleAddCustomApp() {
  if (!customAppForm.value.name.trim() || !customAppForm.value.exe_path.trim()) {
    showToast(t("toast_fill_fields"), "warn");
    return;
  }
  const id = "custom-" + Date.now();
  const probeUrl = customAppForm.value.probe_url.trim();
  const probeUrls = probeUrl ? [probeUrl] : ["https://www.gstatic.com/generate_204"];
  const newApp: TargetAppSpec = {
    id,
    name: customAppForm.value.name.trim(),
    name_en: customAppForm.value.name_en.trim() || customAppForm.value.name.trim(),
    builtin: false,
    exe_path: customAppForm.value.exe_path.trim(),
    proxy_enabled: true,
    probe_urls: probeUrls,
    child_process_patterns: [],
    sidecar_cmdline_regex: "",
    warn_unsaved_on_kill: customAppForm.value.warn_unsaved,
  };
  settings.value.custom_apps.push(newApp);
  saveCurrentSettings();
  addModalOpen.value = false;
  selectNav(id);
  customAppForm.value = { id: "", name: "", name_en: "", exe_path: "", probe_url: "", warn_unsaved: false };
}

function handlePromptRemoveCustomApp(target: TargetAppSpec) {
  const isRunning = appStatuses.value[target.id]?.status && appStatuses.value[target.id]?.status !== "stopped";
  confirmRemoveModal.value = {
    open: true,
    appId: target.id,
    appName: currentLang.value === "en-US" && target.name_en ? target.name_en : target.name,
    isRunning: Boolean(isRunning),
  };
}

async function executeRemoveCustomApp() {
  const targetId = confirmRemoveModal.value.appId;
  settings.value.custom_apps = settings.value.custom_apps.filter((a) => a.id !== targetId);
  delete settings.value.targets[targetId];
  await saveCurrentSettings();
  if (activeNavId.value === targetId) {
    const nextTarget = allTargets.value[0]?.id || "global-proxy";
    selectNav(nextTarget);
  }
  confirmRemoveModal.value = { open: false, appId: "", appName: "", isRunning: false };
  showToast(t("toast_app_removed"), "success");
}

function copyReport() {
  if (!activeDiagReport.value) return;
  navigator.clipboard.writeText(activeDiagReport.value.raw_summary);
  showToast(t("diag_copied"), "success");
}

function statusColor(status?: RunStatus): string {
  switch (status) {
    case "running_proxied":
      return "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border-emerald-500/30";
    case "running_direct":
      return "bg-sky-500/15 text-sky-600 dark:text-sky-400 border-sky-500/30";
    case "running_unproxied":
      return "bg-amber-500/15 text-amber-600 dark:text-amber-400 border-amber-500/30";
    case "restart_needed":
      return "bg-rose-500/15 text-rose-600 dark:text-rose-400 border-rose-500/30";
    default:
      return "bg-slate-500/15 text-slate-500 dark:text-slate-400 border-slate-400/30";
  }
}

function statusText(status?: RunStatus): string {
  switch (status) {
    case "running_proxied":
      return t("status_running_proxied");
    case "running_direct":
      return t("status_running_direct");
    case "running_unproxied":
      return t("status_running_unproxied");
    case "restart_needed":
      return t("status_restart_needed");
    default:
      return t("status_stopped");
  }
}

// Account Management State
const accounts = ref<AccountProfile[]>([]);
const isLoadingAccounts = ref(false);
const isSwitchingAccount = ref(false);
const isRefreshingQuota = ref(false);

const isSupportsAccounts = computed(() => {
  return ["chatgpt", "claude", "antigravity-app", "antigravity-ide", "factory"].includes(currentTarget.value?.id || "");
});

const activeAccount = computed(() => {
  return accounts.value.find((a) => a.is_active) || (accounts.value.length > 0 ? accounts.value[0] : undefined);
});

async function loadAccountsForCurrent() {
  const currentId = currentTarget.value?.id;
  if (!currentId || !isSupportsAccounts.value) {
    accounts.value = [];
    return;
  }
  isLoadingAccounts.value = true;
  try {
    const list = await invoke<AccountProfile[]>("get_accounts", { appId: currentId });
    accounts.value = list;
  } catch (err) {
    console.warn("load accounts failed:", err);
  } finally {
    isLoadingAccounts.value = false;
  }
}

async function handleImportCurrentAccount() {
  const currentId = currentTarget.value?.id;
  if (!currentId) return;
  isLoadingAccounts.value = true;
  try {
    const imported = await invoke<AccountProfile>("import_current_account", {
      appId: currentId,
      customName: null,
    });
    showToast(`${t("active_account_badge")}: ${imported.name} (${imported.email || "OK"})`, "success");
    await loadAccountsForCurrent();
  } catch (err) {
    showToast(`导入失败: ${err}`, "error");
  } finally {
    isLoadingAccounts.value = false;
  }
}

// Confirm Switch Account Modal
const confirmSwitchModal = ref<{
  open: boolean;
  appId: string;
  accountId: string;
  accountName: string;
}>({
  open: false,
  appId: "",
  accountId: "",
  accountName: "",
});

function requestSwitchAccount(account: AccountProfile) {
  const currentId = currentTarget.value?.id;
  if (!currentId) return;
  confirmSwitchModal.value = {
    open: true,
    appId: currentId,
    accountId: account.id,
    accountName: account.name,
  };
}

async function executeSwitchAccount() {
  const { appId, accountId } = confirmSwitchModal.value;
  confirmSwitchModal.value.open = false;
  isSwitchingAccount.value = true;
  try {
    const res = await invoke<AccountSwitchResult>("switch_account", { appId, accountId });
    showToast(res.message, "success");
    await loadAccountsForCurrent();
    await refreshStatuses();
  } catch (err) {
    showToast(`切换账号失败: ${err}`, "error");
  } finally {
    isSwitchingAccount.value = false;
  }
}

async function handleRefreshQuota(accountId?: string) {
  const currentId = currentTarget.value?.id;
  if (!currentId) return;
  isRefreshingQuota.value = true;
  try {
    await invoke<AccountQuota>("refresh_quota", {
      appId: currentId,
      accountId: accountId || null,
    });
    showToast("配额数据已更新", "success");
    await loadAccountsForCurrent();
  } catch (err) {
    showToast(`刷新配额失败: ${err}`, "error");
  } finally {
    isRefreshingQuota.value = false;
  }
}

async function handleDeleteAccount(account: AccountProfile) {
  const currentId = currentTarget.value?.id;
  if (!currentId) return;
  try {
    await invoke("delete_account", { appId: currentId, accountId: account.id });
    showToast(t("account_deleted"), "success");
    await loadAccountsForCurrent();
  } catch (err) {
    showToast(`删除失败: ${err}`, "error");
  }
}

// Add Account Modal
const addAccountModal = ref<{
  open: boolean;
  name: string;
  content: string;
}>({
  open: false,
  name: "",
  content: "",
});

function openAddAccountModal() {
  addAccountModal.value = {
    open: true,
    name: "",
    content: "",
  };
}

async function handleAddAccountSubmit() {
  const currentId = currentTarget.value?.id;
  if (!currentId || !addAccountModal.value.name.trim() || !addAccountModal.value.content.trim()) {
    showToast(t("toast_fill_fields"), "warn");
    return;
  }
  try {
    await invoke<AccountProfile>("add_manual_account", {
      appId: currentId,
      name: addAccountModal.value.name.trim(),
      rawContent: addAccountModal.value.content.trim(),
    });
    showToast("账号添加成功", "success");
    addAccountModal.value.open = false;
    await loadAccountsForCurrent();
  } catch (err) {
    showToast(`添加账号失败: ${err}`, "error");
  }
}

async function handlePrepareClaudeLogin() {
  try {
    const msg = await invoke<string>("prepare_new_claude_login");
    showToast(msg, "info", 5000);
  } catch (err) {
    showToast(`重置会话失败: ${err}`, "error");
  }
}

async function handlePrepareFactoryLogin() {
  try {
    const msg = await invoke<string>("prepare_new_factory_login");
    showToast(msg, "info", 5000);
  } catch (err) {
    showToast(`重置会话失败: ${err}`, "error");
  }
}

function formatCountdown(sec?: number): string {
  if (sec === undefined || sec === null || sec <= 0) return "--";
  const days = Math.floor(sec / 86400);
  const hours = Math.floor((sec % 86400) / 3600);
  const minutes = Math.floor((sec % 3600) / 60);
  const seconds = Math.floor(sec % 60);
  if (days > 0) {
    return `${days}d ${hours}h`;
  }
  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  }
  if (minutes > 0) {
    return `${minutes}m`;
  }
  return `${seconds}s`;
}

watch(
  () => activeNavId.value,
  (newId) => {
    if (["chatgpt", "claude", "antigravity-app", "antigravity-ide", "factory"].includes(newId)) {
      loadAccountsForCurrent();
    }
  }
);

onMounted(async () => {
  await loadInitialData();
  if (["chatgpt", "claude", "antigravity-app", "antigravity-ide", "factory"].includes(activeNavId.value)) {
    await loadAccountsForCurrent();
  }

  statusPollTimer = window.setInterval(refreshStatuses, 2500);
});

onUnmounted(() => {
  if (statusPollTimer) clearInterval(statusPollTimer);
});
</script>

<template>
  <div class="h-screen flex flex-col p-4 max-w-6xl mx-auto space-y-3 overflow-hidden select-none">
    <!-- Toast Notification Container -->
    <div class="fixed top-4 right-4 z-50 flex flex-col gap-2 max-w-sm pointer-events-none">
      <div
        v-for="toast in toasts"
        :key="toast.id"
        :class="[
          'pointer-events-auto px-4 py-2.5 rounded-lg shadow-xl text-xs font-medium flex items-center gap-2 border transition-all duration-200',
          toast.type === 'success' ? 'bg-emerald-600 text-white border-emerald-500 shadow-emerald-600/20' : '',
          toast.type === 'error' ? 'bg-rose-600 text-white border-rose-500 shadow-rose-600/20' : '',
          toast.type === 'warn' ? 'bg-amber-600 text-white border-amber-500 shadow-amber-600/20' : '',
          toast.type === 'info' ? 'bg-slate-800 text-white border-slate-700 shadow-slate-900/30' : '',
        ]"
      >
        <Check v-if="toast.type === 'success'" class="w-4 h-4 shrink-0" />
        <AlertTriangle v-else-if="toast.type === 'warn'" class="w-4 h-4 shrink-0" />
        <AlertCircle v-else-if="toast.type === 'error'" class="w-4 h-4 shrink-0" />
        <Info v-else class="w-4 h-4 shrink-0" />
        <span class="flex-1 leading-normal">{{ toast.message }}</span>
      </div>
    </div>

    <!-- Top Header -->
    <header class="flex items-center justify-between bg-white dark:bg-slate-800 px-4 py-2.5 rounded-xl shadow-sm border border-slate-200/80 dark:border-slate-700/80 shrink-0">
      <div class="flex items-center gap-3">
        <div class="w-9 h-9 rounded-xl bg-gradient-to-tr from-sky-500 to-indigo-600 flex items-center justify-center text-white shadow-md shadow-sky-500/20 shrink-0">
          <Zap class="w-5 h-5" />
        </div>
        <div>
          <h1 class="text-base font-bold tracking-tight text-slate-900 dark:text-white flex items-center gap-2">
            {{ t("app_title") }}
            <span class="text-[10px] px-2 py-0.5 rounded-full bg-sky-100 text-sky-700 dark:bg-sky-900/50 dark:text-sky-300 font-mono">v0.2.8</span>
          </h1>
          <p class="text-[11px] text-slate-500 dark:text-slate-400 leading-tight">{{ t("app_subtitle") }}</p>
        </div>
      </div>

      <div class="flex items-center gap-2.5">
        <!-- Autostart Toggle -->
        <label class="flex items-center gap-1.5 text-xs text-slate-600 dark:text-slate-300 cursor-pointer select-none px-2.5 py-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 hover:bg-slate-100 dark:hover:bg-slate-700 transition">
          <input
            type="checkbox"
            v-model="settings.autostart"
            @change="saveCurrentSettings"
            class="rounded text-sky-600 focus:ring-sky-500 border-slate-300 dark:border-slate-600 dark:bg-slate-700 w-3.5 h-3.5 cursor-pointer"
          />
          <span>{{ t("autostart_label") }}</span>
        </label>

        <!-- Language Switch -->
        <button
          @click="currentLang = currentLang === 'zh-CN' ? 'en-US' : 'zh-CN'; saveCurrentSettings();"
          class="px-2.5 py-1 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 hover:bg-slate-100 dark:hover:bg-slate-700 transition"
        >
          <Globe class="w-3.5 h-3.5 inline mr-1" />
          {{ currentLang === 'zh-CN' ? 'English' : '简体中文' }}
        </button>

        <!-- Theme Switch -->
        <div class="flex items-center rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 p-0.5">
          <button
            @click="applyTheme('light'); saveCurrentSettings();"
            :class="['p-1 rounded-md text-xs transition', settings.theme === 'light' ? 'bg-white dark:bg-slate-600 shadow-sm text-amber-500' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200']"
            title="Light"
          >
            <Sun class="w-3.5 h-3.5" />
          </button>
          <button
            @click="applyTheme('dark'); saveCurrentSettings();"
            :class="['p-1 rounded-md text-xs transition', settings.theme === 'dark' ? 'bg-white dark:bg-slate-600 shadow-sm text-sky-400' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200']"
            title="Dark"
          >
            <Moon class="w-3.5 h-3.5" />
          </button>
          <button
            @click="applyTheme('system'); saveCurrentSettings();"
            :class="['p-1 rounded-md text-xs transition', settings.theme === 'system' ? 'bg-white dark:bg-slate-600 shadow-sm text-indigo-500' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200']"
            title="System"
          >
            <Monitor class="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </header>

    <!-- Main Body: Split Left Sidebar & Right Detail Area -->
    <div class="flex-1 flex gap-4 overflow-hidden min-h-0">
      <!-- Left Navigation Sidebar -->
      <aside class="w-64 flex flex-col bg-white dark:bg-slate-800 rounded-xl shadow-sm border border-slate-200/80 dark:border-slate-700/80 p-3 overflow-hidden select-none shrink-0">
        <!-- Nav Item: Global Proxy Settings -->
        <div class="mb-3">
          <button
            @click="selectNav('global-proxy')"
            :class="[
              'w-full flex items-center justify-between px-3 py-2.5 rounded-lg text-xs font-semibold transition border text-left',
              activeNavId === 'global-proxy'
                ? 'bg-sky-50 dark:bg-sky-950/50 text-sky-700 dark:text-sky-300 border-sky-300 dark:border-sky-800 shadow-sm'
                : 'border-transparent text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/60'
            ]"
          >
            <div class="flex items-center gap-2.5">
              <Network class="w-4 h-4 text-sky-500 shrink-0" />
              <span>{{ t("nav_global_proxy") }}</span>
            </div>
            <span class="text-[10px] font-mono font-normal px-1.5 py-0.5 rounded bg-slate-100 dark:bg-slate-700 text-slate-500 dark:text-slate-400">
              :{{ settings.proxy.port }}
            </span>
          </button>
        </div>

        <!-- Section Title: AI Apps -->
        <div class="flex items-center justify-between px-1 mb-2">
          <span class="text-[11px] font-bold text-slate-400 uppercase tracking-wider">{{ t("nav_apps_group") }}</span>
          <span class="text-[10px] text-slate-400 font-mono">{{ allTargets.length }}</span>
        </div>

        <!-- Apps List Container -->
        <div class="flex-1 overflow-y-auto space-y-1 pr-0.5 min-h-0">
          <div
            v-for="target in allTargets"
            :key="target.id"
            tabindex="0"
            @click="selectNav(target.id)"
            @keydown.enter="selectNav(target.id)"
            :class="[
              'group w-full flex items-center justify-between px-3 py-2 rounded-lg text-xs transition border cursor-pointer select-none outline-none focus:ring-1 focus:ring-sky-500',
              activeNavId === target.id
                ? 'bg-sky-50 dark:bg-sky-950/50 text-sky-700 dark:text-sky-300 border-sky-300 dark:border-sky-800 shadow-sm font-semibold'
                : 'border-transparent text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/60 font-medium'
            ]"
          >
            <!-- Left: Status dot + App Name -->
            <div class="flex items-center gap-2 min-w-0 pr-1">
              <span
                :class="[
                  'w-2 h-2 rounded-full shrink-0',
                  appStatuses[target.id]?.status === 'running_proxied' ? 'bg-emerald-500 ring-2 ring-emerald-500/20' :
                  appStatuses[target.id]?.status === 'running_direct' ? 'bg-sky-500 ring-2 ring-sky-500/20' :
                  appStatuses[target.id]?.status === 'running_unproxied' ? 'bg-amber-500 ring-2 ring-amber-500/20 animate-pulse' :
                  appStatuses[target.id]?.status === 'restart_needed' ? 'bg-rose-500 ring-2 ring-rose-500/20 animate-pulse' :
                  'bg-slate-300 dark:bg-slate-600'
                ]"
              ></span>
              <span class="truncate">
                {{ currentLang === 'en-US' && target.name_en ? target.name_en : target.name }}
              </span>
            </div>

            <!-- Right: Status Tag (default) OR Quick Action Button (on Hover) -->
            <div class="shrink-0 flex items-center">
              <!-- Default state badge -->
              <span
                :class="[
                  'group-hover:hidden text-[10px] font-mono px-1.5 py-0.5 rounded border transition',
                  statusColor(appStatuses[target.id]?.status)
                ]"
              >
                {{ statusText(appStatuses[target.id]?.status) }}
              </span>

              <!-- Hover state quick action button -->
              <div class="hidden group-hover:flex group-focus-within:flex items-center gap-1" @click.stop>
                <!-- Stop button if running -->
                <button
                  v-if="appStatuses[target.id]?.status && appStatuses[target.id]?.status !== 'stopped'"
                  @click.stop="handleStop(target)"
                  class="p-1 rounded bg-rose-100 hover:bg-rose-200 dark:bg-rose-950/60 dark:hover:bg-rose-900 text-rose-600 dark:text-rose-400 transition"
                  :title="t('quick_stop')"
                >
                  <Square class="w-3 h-3 fill-current" />
                </button>

                <!-- Launch / Restart button -->
                <button
                  v-else
                  @click.stop="handleLaunch(target)"
                  :disabled="Boolean(launchingIds[target.id])"
                  class="p-1 rounded bg-emerald-100 hover:bg-emerald-200 dark:bg-emerald-950/60 dark:hover:bg-emerald-900 text-emerald-600 dark:text-emerald-400 transition disabled:opacity-50 disabled:cursor-not-allowed"
                  :title="t('quick_launch')"
                >
                  <RotateCw v-if="launchingIds[target.id]" class="w-3 h-3 animate-spin" />
                  <Play v-else class="w-3 h-3 fill-current" />
                </button>
              </div>
            </div>
          </div>
        </div>

        <!-- Bottom Add Custom App Button -->
        <div class="pt-3 border-t border-slate-100 dark:border-slate-700/60 mt-2 shrink-0">
          <button
            @click="addModalOpen = true"
            class="w-full py-2 px-3 text-xs font-medium text-sky-600 dark:text-sky-400 hover:bg-sky-50 dark:hover:bg-sky-950/40 rounded-lg flex items-center justify-center gap-1.5 border border-dashed border-sky-300 dark:border-sky-800 transition"
          >
            <Plus class="w-3.5 h-3.5" />
            <span>{{ t("add_custom_app") }}</span>
          </button>
        </div>
      </aside>

      <!-- Right Main Content Panel -->
      <main class="flex-1 bg-white dark:bg-slate-800 rounded-xl shadow-sm border border-slate-200/80 dark:border-slate-700/80 p-5 overflow-y-auto flex flex-col min-h-0 space-y-4">
        
        <!-- VIEW 1: Global Proxy Settings -->
        <template v-if="activeNavId === 'global-proxy'">
          <!-- Header -->
          <div class="flex items-center justify-between border-b border-slate-100 dark:border-slate-700/60 pb-3 shrink-0">
            <div class="flex items-center gap-2.5">
              <div class="w-8 h-8 rounded-lg bg-sky-50 dark:bg-sky-950/60 flex items-center justify-center text-sky-600 dark:text-sky-400">
                <ShieldCheck class="w-5 h-5" />
              </div>
              <div>
                <h2 class="text-base font-bold text-slate-900 dark:text-white">{{ t("global_proxy_title") }}</h2>
                <p class="text-xs text-slate-500 dark:text-slate-400">{{ t("global_proxy_desc") }}</p>
              </div>
            </div>
            <div class="flex items-center gap-2">
              <span v-if="saveSuccess" class="text-xs text-emerald-600 dark:text-emerald-400 font-medium flex items-center gap-1">
                <Check class="w-3.5 h-3.5" /> {{ t("config_saved") }}
              </span>
              <button
                @click="saveCurrentSettings"
                class="px-4 py-1.5 text-xs font-medium rounded-lg bg-sky-600 text-white hover:bg-sky-700 transition shadow-sm"
              >
                {{ t("save_proxy") }}
              </button>
            </div>
          </div>

          <!-- Proxy Protocol / Host / Port Form -->
          <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
            <!-- Scheme -->
            <div>
              <label class="block text-xs font-medium text-slate-600 dark:text-slate-300 mb-1.5">{{ t("proxy_scheme") }}</label>
              <select
                v-model="settings.proxy.scheme"
                class="w-full text-xs rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2.5 focus:ring-2 focus:ring-sky-500 outline-none"
              >
                <option value="http">HTTP</option>
                <option value="socks5">SOCKS5</option>
              </select>
            </div>

            <!-- Host -->
            <div>
              <label class="block text-xs font-medium text-slate-600 dark:text-slate-300 mb-1.5">{{ t("proxy_host") }}</label>
              <input
                v-model="settings.proxy.host"
                type="text"
                placeholder="127.0.0.1"
                class="w-full text-xs font-mono rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2.5 focus:ring-2 focus:ring-sky-500 outline-none"
              />
            </div>

            <!-- Port with Scanner -->
            <div class="relative">
              <div class="flex items-center justify-between mb-1.5">
                <label class="text-xs font-medium text-slate-600 dark:text-slate-300">{{ t("proxy_port") }}</label>
                <button
                  @click="handleScanPorts"
                  :disabled="isScanning"
                  class="text-xs text-sky-600 dark:text-sky-400 hover:underline flex items-center gap-1 font-medium disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  <Search :class="['w-3 h-3', isScanning ? 'animate-spin' : '']" />
                  {{ isScanning ? t("scanning_ports") : t("scan_ports") }}
                </button>
              </div>
              <input
                v-model.number="settings.proxy.port"
                type="number"
                placeholder="7890"
                class="w-full text-xs font-mono rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2.5 focus:ring-2 focus:ring-sky-500 outline-none"
              />

              <!-- Scanned Ports Dropdown -->
              <div
                v-if="showPortDropdown && scannedPorts.length > 0"
                class="absolute z-20 left-0 right-0 mt-1 bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-lg shadow-lg overflow-hidden py-1"
              >
                <div class="text-xs px-3 py-1 font-semibold text-slate-400 border-b border-slate-100 dark:border-slate-700">{{ t("scanned_ports_title") }}</div>
                <button
                  v-for="p in scannedPorts"
                  :key="p"
                  @click="selectScannedPort(p)"
                  class="w-full text-left px-3 py-1.5 text-xs text-slate-700 dark:text-slate-200 hover:bg-sky-50 dark:hover:bg-slate-700/60 flex items-center justify-between"
                >
                  <span class="font-mono font-medium">{{ p }}</span>
                  <span class="text-emerald-500 text-[10px]">{{ t("port_online") }}</span>
                </button>
              </div>
            </div>
          </div>

          <!-- NO_PROXY Bypass Collapsible -->
          <div class="border-t border-slate-100 dark:border-slate-700/40 pt-4">
            <button
              @click="showNoProxy = !showNoProxy"
              class="flex items-center gap-1.5 text-xs text-slate-600 dark:text-slate-400 hover:text-slate-800 dark:hover:text-slate-200 transition font-medium mb-2"
            >
              <component :is="showNoProxy ? ChevronUp : ChevronDown" class="w-3.5 h-3.5" />
              {{ t("no_proxy_bypass") }}
            </button>

            <div v-if="showNoProxy" class="space-y-1">
              <textarea
                v-model="settings.proxy.no_proxy"
                rows="3"
                class="w-full text-xs font-mono rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2.5 focus:ring-2 focus:ring-sky-500 outline-none"
              ></textarea>
              <p class="text-[11px] text-slate-400">{{ t("no_proxy_tip") }}</p>
            </div>
          </div>
        </template>

        <!-- VIEW 2: Selected App Details -->
        <template v-else-if="currentTarget">
          <!-- App Detail Header & Primary Action Toolbar -->
          <div class="flex items-center justify-between border-b border-slate-100 dark:border-slate-700/60 pb-3 shrink-0">
            <div class="flex items-center gap-3">
              <div>
                <div class="flex items-center gap-2">
                  <h2 class="text-lg font-bold text-slate-900 dark:text-white">
                    {{ currentLang === 'en-US' && currentTarget.name_en ? currentTarget.name_en : currentTarget.name }}
                  </h2>
                  <span
                    :class="['text-xs px-2.5 py-0.5 rounded-full border font-medium flex items-center gap-1.5', statusColor(appStatuses[currentTarget.id]?.status)]"
                  >
                    <span class="w-1.5 h-1.5 rounded-full bg-current animate-pulse"></span>
                    {{ statusText(appStatuses[currentTarget.id]?.status) }}
                  </span>
                  <span v-if="currentTarget.builtin" class="text-[10px] px-2 py-0.5 rounded bg-slate-100 dark:bg-slate-700 text-slate-500 font-mono">Builtin</span>
                  <span v-else class="text-[10px] px-2 py-0.5 rounded bg-indigo-50 dark:bg-indigo-950/60 text-indigo-600 dark:text-indigo-400 border border-indigo-200 dark:border-indigo-800 font-mono">Custom</span>
                </div>
                <p class="text-[11px] text-slate-400 mt-0.5">ID: {{ currentTarget.id }}</p>
              </div>
            </div>

            <!-- Actions (Launch, Stop, Diag, Shortcut, Delete) -->
            <div class="flex items-center gap-2">
              <!-- Launch / Restart -->
              <button
                v-if="appStatuses[currentTarget.id]?.status === 'restart_needed' || appStatuses[currentTarget.id]?.status === 'running_unproxied'"
                @click="handleLaunch(currentTarget)"
                :disabled="Boolean(launchingIds[currentTarget.id])"
                class="px-4 py-2 text-xs font-semibold rounded-lg bg-rose-600 text-white hover:bg-rose-700 disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-1.5 shadow-sm transition"
              >
                <RotateCw :class="['w-3.5 h-3.5', launchingIds[currentTarget.id] ? 'animate-spin' : '']" />
                {{ t("btn_restart") }}
              </button>
              <button
                v-else
                @click="handleLaunch(currentTarget)"
                :disabled="Boolean(launchingIds[currentTarget.id])"
                class="px-4 py-2 text-xs font-semibold rounded-lg bg-emerald-600 text-white hover:bg-emerald-700 disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-1.5 shadow-sm transition"
              >
                <RotateCw v-if="launchingIds[currentTarget.id]" class="w-3.5 h-3.5 animate-spin" />
                <Play v-else class="w-3.5 h-3.5" />
                {{ t("btn_launch") }}
              </button>

              <!-- Stop Button -->
              <button
                v-if="appStatuses[currentTarget.id]?.status !== 'stopped'"
                @click="handleStop(currentTarget)"
                class="px-3.5 py-2 text-xs font-medium rounded-lg border border-rose-300 dark:border-rose-800/80 text-rose-600 dark:text-rose-400 hover:bg-rose-50 dark:hover:bg-rose-950/40 flex items-center gap-1.5 transition"
              >
                <Square class="w-3.5 h-3.5" />
                {{ t("btn_stop") }}
              </button>

              <!-- Diagnostics Button -->
              <button
                @click="handleRunDiagnostics(currentTarget)"
                :disabled="isRunningDiag"
                class="px-3 py-2 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 flex items-center gap-1.5 transition disabled:opacity-50 disabled:cursor-not-allowed"
              >
                <Activity :class="['w-3.5 h-3.5 text-sky-500', isRunningDiag ? 'animate-spin' : '']" />
                {{ isRunningDiag ? t("diag_running") : t("btn_diagnostics") }}
              </button>

              <!-- Desktop Shortcut -->
              <button
                @click="handleCreateShortcut(currentTarget)"
                class="px-3 py-2 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 flex items-center gap-1.5 transition"
                :title="t('btn_shortcut_tip')"
              >
                <ExternalLink class="w-3.5 h-3.5 text-indigo-500" />
                {{ t("btn_shortcut") }}
              </button>

              <!-- Delete Custom App -->
              <button
                v-if="!currentTarget.builtin"
                @click="handlePromptRemoveCustomApp(currentTarget)"
                class="p-2 rounded-lg text-slate-400 hover:text-rose-600 hover:bg-rose-50 dark:hover:bg-rose-950/40 transition"
                :title="t('btn_remove_app')"
              >
                <Trash2 class="w-4 h-4" />
              </button>
            </div>
          </div>

          <!-- Card 1: 路由代理模式 (Routing Mode) -->
          <div class="p-4 rounded-xl border border-slate-200/80 dark:border-slate-700/80 bg-slate-50/50 dark:bg-slate-900/30 space-y-3">
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-2 text-xs font-bold text-slate-800 dark:text-slate-200">
                <Sliders class="w-4 h-4 text-sky-500" />
                <span>{{ t("section_routing_mode") }}</span>
              </div>

              <!-- Proxied vs Direct Segmented Toggle -->
              <div class="flex items-center bg-slate-200/80 dark:bg-slate-900 p-0.5 rounded-lg border border-slate-300/80 dark:border-slate-700">
                <button
                  @click="!currentTarget.proxy_enabled && toggleProxyMode(currentTarget)"
                  :class="['px-3 py-1 text-xs font-medium rounded-md transition', currentTarget.proxy_enabled ? 'bg-sky-600 text-white shadow-sm' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200']"
                >
                  {{ t("mode_proxied") }}
                </button>
                <button
                  @click="currentTarget.proxy_enabled && toggleProxyMode(currentTarget)"
                  :class="['px-3 py-1 text-xs font-medium rounded-md transition', !currentTarget.proxy_enabled ? 'bg-slate-600 text-white shadow-sm' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200']"
                >
                  {{ t("mode_direct") }}
                </button>
              </div>
            </div>

            <p class="text-xs text-slate-500 dark:text-slate-400 leading-relaxed">
              {{ currentTarget.proxy_enabled ? t("mode_proxied_desc") : t("mode_direct_desc") }}
              <span v-if="currentTarget.proxy_enabled" class="font-mono text-sky-600 dark:text-sky-400 ml-1">
                ({{ settings.proxy.scheme }}://{{ settings.proxy.host }}:{{ settings.proxy.port }})
              </span>
            </p>
          </div>

          <!-- Card 2: 目标程序路径 (Executable Path) -->
          <div class="p-4 rounded-xl border border-slate-200/80 dark:border-slate-700/80 bg-slate-50/50 dark:bg-slate-900/30 space-y-3">
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-2 text-xs font-bold text-slate-800 dark:text-slate-200">
                <Folder class="w-4 h-4 text-indigo-500" />
                <span>{{ t("section_exe_path") }}</span>
              </div>
              <button
                v-if="currentTarget.builtin"
                @click="handleAutoDetect(currentTarget.id)"
                class="px-2.5 py-1 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-600 dark:text-slate-300 flex items-center gap-1 transition shadow-xs"
              >
                <Search class="w-3.5 h-3.5" /> {{ t("btn_detect") }}
              </button>
            </div>

            <div class="flex items-center gap-2">
              <input
                v-model="currentTarget.exe_path"
                type="text"
                :placeholder="t('path_not_found')"
                class="flex-1 text-xs font-mono rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900/60 p-2.5 focus:ring-2 focus:ring-sky-500 outline-none text-slate-700 dark:text-slate-200"
                @change="onTargetExeChange(currentTarget)"
              />
            </div>

            <!-- Probe URL Display -->
            <div v-if="currentTarget.probe_urls?.length" class="flex items-center gap-2 text-[11px] text-slate-400">
              <span>{{ t("probe_endpoint") }}:</span>
              <span class="font-mono text-slate-500 dark:text-slate-400">{{ currentTarget.probe_urls[0] }}</span>
            </div>
          </div>

          <!-- Card: 多账号与配额管理 (Account & Quota Management) -->
          <div
            v-if="isSupportsAccounts"
            class="p-4 rounded-xl border border-sky-200/80 dark:border-sky-900/60 bg-gradient-to-b from-sky-50/40 to-white dark:from-slate-900/60 dark:to-slate-900/30 space-y-4 shadow-xs"
          >
            <!-- Card Header: Title & Actions -->
            <div class="flex items-center justify-between flex-wrap gap-2">
              <div class="flex items-center gap-2">
                <div class="w-7 h-7 rounded-lg bg-sky-500/10 dark:bg-sky-400/10 flex items-center justify-center text-sky-600 dark:text-sky-400">
                  <Users class="w-4 h-4" />
                </div>
                <div>
                  <div class="flex items-center gap-2">
                    <span class="text-xs font-bold text-slate-800 dark:text-slate-200">{{ t("section_accounts") }}</span>
                    <span class="text-[10px] font-mono px-1.5 py-0.5 rounded-full bg-sky-100 dark:bg-sky-950 text-sky-700 dark:text-sky-300">
                      {{ accounts.length }}
                    </span>
                  </div>
                  <p class="text-[11px] text-slate-400 leading-none mt-0.5">{{ t("section_accounts_desc") }}</p>
                </div>
              </div>

              <!-- Top Toolbar Actions -->
              <div class="flex items-center gap-1.5 flex-wrap">
                <!-- Refresh Quota -->
                <button
                  @click="handleRefreshQuota()"
                  :disabled="isRefreshingQuota || accounts.length === 0"
                  class="px-2.5 py-1 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 flex items-center gap-1 transition shadow-xs disabled:opacity-50 disabled:cursor-not-allowed"
                  :title="t('btn_refresh_quota')"
                >
                  <RefreshCw :class="['w-3.5 h-3.5 text-sky-500', isRefreshingQuota ? 'animate-spin' : '']" />
                  <span>{{ t("btn_refresh_quota") }}</span>
                </button>

                <!-- Import Active Account -->
                <button
                  @click="handleImportCurrentAccount()"
                  :disabled="isLoadingAccounts"
                  class="px-2.5 py-1 text-xs font-semibold rounded-lg bg-sky-600 hover:bg-sky-700 text-white flex items-center gap-1 transition shadow-xs disabled:opacity-50"
                  :title="t('btn_import_active_account')"
                >
                  <RotateCw v-if="isLoadingAccounts" class="w-3.5 h-3.5 animate-spin" />
                  <UserCheck v-else class="w-3.5 h-3.5" />
                  <span>{{ t("btn_import_active_account") }}</span>
                </button>

                <!-- Add Account (Manual) -->
                <button
                  @click="openAddAccountModal()"
                  class="px-2.5 py-1 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 flex items-center gap-1 transition shadow-xs"
                >
                  <PlusCircle class="w-3.5 h-3.5 text-indigo-500" />
                  <span>{{ t("btn_add_account") }}</span>
                </button>

                <!-- Reset Claude Session (Claude only) -->
                <button
                  v-if="currentTarget.id === 'claude'"
                  @click="handlePrepareClaudeLogin()"
                  class="px-2.5 py-1 text-[11px] font-medium rounded-lg border border-amber-300 dark:border-amber-800/60 bg-amber-50 dark:bg-amber-950/40 text-amber-700 dark:text-amber-300 hover:bg-amber-100 dark:hover:bg-amber-900/60 flex items-center gap-1 transition"
                  :title="t('claude_reset_session_tip')"
                >
                  <Key class="w-3 h-3 text-amber-500" />
                  <span>{{ t("claude_reset_session_btn") }}</span>
                </button>

                <!-- Reset Factory Session (Factory only) -->
                <button
                  v-if="currentTarget.id === 'factory'"
                  @click="handlePrepareFactoryLogin()"
                  class="px-2.5 py-1 text-[11px] font-medium rounded-lg border border-indigo-300 dark:border-indigo-800/60 bg-indigo-50 dark:bg-indigo-950/40 text-indigo-700 dark:text-indigo-300 hover:bg-indigo-100 dark:hover:bg-indigo-900/60 flex items-center gap-1 transition cursor-pointer"
                  :title="t('factory_reset_session_tip')"
                >
                  <Key class="w-3 h-3 text-indigo-500" />
                  <span>{{ t("factory_reset_session_btn") }}</span>
                </button>
              </div>
            </div>

            <!-- Active Account Card & Quota Display -->
            <div v-if="activeAccount" class="space-y-3">
              <!-- Active Account Banner -->
              <div class="p-3 rounded-lg border border-emerald-200/80 dark:border-emerald-800/60 bg-emerald-50/40 dark:bg-emerald-950/20 flex items-center justify-between flex-wrap gap-2">
                <div class="flex items-center gap-3">
                  <div class="w-8 h-8 rounded-full bg-emerald-600 text-white font-bold flex items-center justify-center text-xs shadow-xs">
                    {{ activeAccount.name.slice(0, 2).toUpperCase() }}
                  </div>
                  <div>
                    <div class="flex items-center gap-2 flex-wrap">
                      <span class="text-xs font-bold text-slate-800 dark:text-slate-100">{{ activeAccount.name }}</span>
                      <span class="text-[10px] font-medium px-2 py-0.5 rounded-full bg-emerald-100 dark:bg-emerald-900/60 text-emerald-700 dark:text-emerald-300 border border-emerald-300 dark:border-emerald-700/80">
                        {{ t("active_account_badge") }}
                      </span>
                      <span v-if="activeAccount.plan_type" class="text-[10px] font-semibold px-2 py-0.5 rounded bg-indigo-100 dark:bg-indigo-950 text-indigo-700 dark:text-indigo-300 border border-indigo-200 dark:border-indigo-800">
                        {{ activeAccount.plan_type }}
                      </span>

                      <!-- Factory Session Token Expiry Badge -->
                      <template v-if="activeAccount.quota?.extra_info?.['token_expires_at']">
                        <span
                          v-if="activeAccount.quota.extra_info['token_is_expired'] === 'true'"
                          class="text-[10px] font-medium px-2 py-0.5 rounded-full bg-rose-100 dark:bg-rose-950/60 text-rose-700 dark:text-rose-300 border border-rose-300 dark:border-rose-800/80 flex items-center gap-1"
                          :title="`令牌到期时间: ${activeAccount.quota.extra_info['token_expires_at']}`"
                        >
                          <AlertCircle class="w-3 h-3 text-rose-500 shrink-0" />
                          <span>{{ t("token_status_expired") }}</span>
                        </span>
                        <span
                          v-else
                          class="text-[10px] font-medium px-2 py-0.5 rounded-full bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300 border border-slate-300 dark:border-slate-700 flex items-center gap-1"
                          :title="`令牌到期时间: ${activeAccount.quota.extra_info['token_expires_at']}`"
                        >
                          <Key class="w-3 h-3 text-slate-400 shrink-0" />
                          <span>{{ t("token_status_valid") }}: {{ activeAccount.quota.extra_info['token_status'] }}</span>
                        </span>
                      </template>

                      <!-- ChatGPT Subscription Expiry Badge -->
                      <template v-if="activeAccount.quota?.extra_info?.['subscription_expires_at']">
                        <span
                          v-if="activeAccount.quota.extra_info['subscription_is_expired'] === 'true'"
                          class="text-[10px] font-medium px-2 py-0.5 rounded-full bg-rose-100 dark:bg-rose-950/60 text-rose-700 dark:text-rose-300 border border-rose-300 dark:border-rose-800/80 flex items-center gap-1"
                          :title="`订阅到期时间: ${activeAccount.quota.extra_info['subscription_expires_at']}`"
                        >
                          <AlertCircle class="w-3 h-3 text-rose-500 shrink-0" />
                          <span>{{ t("subscription_expired") }}: {{ activeAccount.quota.extra_info['subscription_expires_at'].slice(0, 10) }}</span>
                        </span>
                        <span
                          v-else
                          class="text-[10px] font-medium px-2 py-0.5 rounded-full bg-indigo-50 dark:bg-indigo-950/40 text-indigo-700 dark:text-indigo-300 border border-indigo-200 dark:border-indigo-800/60 flex items-center gap-1"
                          :title="`订阅到期时间: ${activeAccount.quota.extra_info['subscription_expires_at']}`"
                        >
                          <Calendar class="w-3 h-3 text-indigo-500 shrink-0" />
                          <span>{{ t("subscription_valid_until") }}: {{ activeAccount.quota.extra_info['subscription_expires_at'].slice(0, 10) }}</span>
                        </span>
                      </template>
                    </div>
                    <p class="text-[11px] text-slate-500 dark:text-slate-400 mt-0.5 font-mono">
                      {{ activeAccount.email || activeAccount.account_id || "本地凭证" }}
                    </p>
                  </div>
                </div>

                <div class="flex items-center gap-3 text-[11px] text-slate-400 font-mono">
                  <span v-if="activeAccount.quota?.fetched_at">
                    {{ t("btn_refresh_quota") }}: {{ activeAccount.quota.fetched_at }}
                  </span>
                </div>
              </div>

              <!-- Quota Windows Grid -->
              <div v-if="activeAccount.quota?.windows?.length" class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
                <div
                  v-for="(w, idx) in activeAccount.quota.windows"
                  :key="idx"
                  class="p-3.5 rounded-lg border border-slate-200/80 dark:border-slate-700/80 bg-white dark:bg-slate-800/80 flex flex-col justify-between space-y-2.5 shadow-2xs hover:border-slate-300 dark:hover:border-slate-600 transition"
                >
                  <div class="flex flex-wrap items-baseline justify-between gap-x-2 gap-y-1">
                    <span class="text-xs font-bold text-slate-800 dark:text-slate-100 break-words leading-snug" :title="w.name">
                      {{ w.name }}
                    </span>
                    <span v-if="w.used_percent !== undefined && w.used_percent !== null && w.remaining_percent !== undefined && w.remaining_percent !== null" class="text-xs font-bold font-mono shrink-0 whitespace-nowrap ml-auto">
                      <span class="text-amber-600 dark:text-amber-400">{{ w.used_percent }}% {{ t("quota_used") }}</span>
                      <span class="text-slate-300 dark:text-slate-600 mx-1">/</span>
                      <span class="text-sky-600 dark:text-sky-400">{{ w.remaining_percent }}% {{ t("quota_remaining") }}</span>
                    </span>
                    <span v-else-if="w.remaining_percent !== undefined && w.remaining_percent !== null" class="text-xs font-bold font-mono text-sky-600 dark:text-sky-400 shrink-0 whitespace-nowrap pt-0.5">
                      {{ w.remaining_percent }}% {{ t("quota_remaining") }}
                    </span>
                    <span v-else-if="w.used_percent !== undefined && w.used_percent !== null" class="text-xs font-bold font-mono text-amber-600 dark:text-amber-400 shrink-0 whitespace-nowrap pt-0.5">
                      {{ w.used_percent }}% {{ t("quota_used") }}
                    </span>
                  </div>

                  <!-- Progress Bar -->
                  <div v-if="w.remaining_percent != null || w.used_percent != null" class="w-full bg-slate-100 dark:bg-slate-700 h-2 rounded-full overflow-hidden">
                    <div
                      class="h-full rounded-full transition-all duration-300"
                      :class="[
                        (w.remaining_percent ?? (100 - (w.used_percent || 0))) > 40
                          ? 'bg-emerald-500'
                          : (w.remaining_percent ?? (100 - (w.used_percent || 0))) > 15
                          ? 'bg-amber-500'
                          : 'bg-rose-500'
                      ]"
                      :style="{ width: `${w.remaining_percent ?? (100 - (w.used_percent || 0))}%` }"
                    ></div>
                  </div>

                  <!-- Reset Information -->
                  <div class="flex items-center justify-between text-[11px] text-slate-400 pt-1.5 border-t border-slate-100 dark:border-slate-700/50">
                    <div class="flex items-center gap-1.5 text-slate-500 dark:text-slate-400" :title="w.resets_at ? `重置/到期时间: ${w.resets_at}` : undefined">
                      <Clock class="w-3.5 h-3.5 text-slate-400 shrink-0" />
                      <span v-if="w.reset_in_seconds && w.reset_in_seconds > 0" class="font-mono text-sky-600 dark:text-sky-400 font-semibold">
                        {{ formatCountdown(w.reset_in_seconds) }}
                      </span>
                      <span v-else>--</span>
                    </div>

                    <span v-if="w.resets_at" class="font-mono text-[10px] text-slate-400 shrink-0 ml-auto whitespace-nowrap" :title="`到期/重置时间: ${w.resets_at}`">
                      {{ w.resets_at }}
                    </span>
                  </div>

                  <!-- Detail Description -->
                  <p v-if="w.detail" class="text-[11px] text-slate-500 dark:text-slate-400 leading-snug line-clamp-2" :title="w.detail">
                    {{ w.detail }}
                  </p>
                </div>
              </div>
            </div>

            <!-- Empty State if no active account -->
            <div
              v-else-if="accounts.length === 0"
              class="py-6 px-4 rounded-lg border border-dashed border-slate-300 dark:border-slate-700 text-center space-y-2 bg-white/60 dark:bg-slate-800/40"
            >
              <Users class="w-8 h-8 text-slate-300 dark:text-slate-600 mx-auto" />
              <p class="text-xs text-slate-500 dark:text-slate-400 max-w-md mx-auto leading-relaxed">
                {{ t("no_accounts_tip") }}
              </p>
              <button
                @click="handleImportCurrentAccount()"
                class="px-3.5 py-1.5 text-xs font-semibold rounded-lg bg-sky-600 hover:bg-sky-700 text-white inline-flex items-center gap-1.5 shadow-sm transition"
              >
                <UserCheck class="w-3.5 h-3.5" />
                {{ t("btn_import_active_account") }}
              </button>
            </div>

            <!-- Saved Accounts List Section -->
            <div v-if="accounts.length > 0" class="space-y-2 pt-2 border-t border-slate-200/60 dark:border-slate-700/60">
              <div class="flex items-center justify-between">
                <span class="text-[11px] font-bold text-slate-500 dark:text-slate-400 uppercase tracking-wider">
                  {{ t("saved_accounts_list") }} ({{ accounts.length }})
                </span>
              </div>

              <div class="space-y-1.5">
                <div
                  v-for="acct in accounts"
                  :key="acct.id"
                  :class="[
                    'p-2.5 rounded-lg border text-xs flex items-center justify-between transition',
                    acct.is_active
                      ? 'bg-sky-50/60 dark:bg-sky-950/30 border-sky-300 dark:border-sky-800/80 shadow-xs'
                      : 'bg-white dark:bg-slate-800/80 border-slate-200/80 dark:border-slate-700/80 hover:bg-slate-50 dark:hover:bg-slate-750'
                  ]"
                >
                  <div class="flex items-center gap-2.5 min-w-0 pr-2">
                    <!-- Dot / Avatar -->
                    <span
                      :class="[
                        'w-2 h-2 rounded-full shrink-0',
                        acct.is_active ? 'bg-emerald-500 ring-2 ring-emerald-200 dark:ring-emerald-900' : 'bg-slate-300 dark:bg-slate-600'
                      ]"
                    ></span>

                    <div class="min-w-0">
                      <div class="flex items-center gap-1.5">
                        <span class="font-semibold text-slate-800 dark:text-slate-200 truncate">{{ acct.name }}</span>
                        <span v-if="acct.is_active" class="text-[9px] font-medium px-1.5 py-0.2 rounded bg-emerald-100 dark:bg-emerald-950 text-emerald-700 dark:text-emerald-300 border border-emerald-300 dark:border-emerald-800">
                          {{ t("active_account_badge") }}
                        </span>
                        <span v-if="acct.plan_type" class="text-[9px] px-1.5 py-0.2 rounded bg-indigo-50 dark:bg-indigo-950/60 text-indigo-600 dark:text-indigo-400 border border-indigo-200 dark:border-indigo-800 font-mono">
                          {{ acct.plan_type }}
                        </span>
                        <span v-if="acct.quota?.extra_info?.['token_is_expired'] === 'true' || acct.quota?.extra_info?.['subscription_is_expired'] === 'true'" class="text-[9px] font-medium px-1.5 py-0.2 rounded bg-rose-100 dark:bg-rose-950 text-rose-700 dark:text-rose-300 border border-rose-300 dark:border-rose-800">
                          {{ t("badge_expired") }}
                        </span>
                      </div>
                      <p class="text-[11px] text-slate-400 font-mono truncate">
                        {{ acct.email || acct.account_id || t("unbound_email") }}
                      </p>
                    </div>
                  </div>

                  <!-- Actions -->
                  <div class="flex items-center gap-1.5 shrink-0">
                    <button
                      v-if="!acct.is_active"
                      @click="requestSwitchAccount(acct)"
                      :disabled="isSwitchingAccount"
                      class="px-3 py-1 text-xs font-semibold rounded-lg bg-sky-600 hover:bg-sky-700 text-white flex items-center gap-1 shadow-xs transition disabled:opacity-50"
                    >
                      <ArrowRightLeft class="w-3 h-3" />
                      <span>{{ t("btn_switch") }}</span>
                    </button>

                    <button
                      @click="handleRefreshQuota(acct.id)"
                      :disabled="isRefreshingQuota"
                      class="p-1.5 rounded-lg border border-slate-200 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-500 hover:text-slate-800 dark:hover:text-slate-200 transition"
                      :title="t('btn_refresh_quota')"
                    >
                      <RefreshCw class="w-3.5 h-3.5" />
                    </button>

                    <button
                      v-if="!acct.is_active"
                      @click="handleDeleteAccount(acct)"
                      class="p-1.5 rounded-lg text-slate-400 hover:text-rose-600 hover:bg-rose-50 dark:hover:bg-rose-950/40 transition"
                      :title="t('btn_delete')"
                    >
                      <Trash2 class="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>


          <!-- Card 3: 运行监控与进程信息 (Process Monitoring & Runtime Info) -->
          <div class="p-4 rounded-xl border border-slate-200/80 dark:border-slate-700/80 bg-slate-50/50 dark:bg-slate-900/30 space-y-3">
            <div class="flex items-center gap-2 text-xs font-bold text-slate-800 dark:text-slate-200">
              <Cpu class="w-4 h-4 text-emerald-500" />
              <span>{{ t("section_process_info") }}</span>
            </div>

            <div v-if="appStatuses[currentTarget.id]?.status && appStatuses[currentTarget.id]?.status !== 'stopped'" class="space-y-2.5">
              <div class="grid grid-cols-2 gap-3 text-xs">
                <div class="p-2.5 rounded-lg bg-white dark:bg-slate-800 border border-slate-200/60 dark:border-slate-700/60">
                  <span class="text-slate-400 block text-[11px] mb-1">{{ t("process_pid") }}</span>
                  <span class="font-mono font-bold text-slate-800 dark:text-slate-200">
                    {{ appStatuses[currentTarget.id]?.pids?.join(", ") || "N/A" }}
                  </span>
                </div>
                <div class="p-2.5 rounded-lg bg-white dark:bg-slate-800 border border-slate-200/60 dark:border-slate-700/60">
                  <span class="text-slate-400 block text-[11px] mb-1">{{ t("process_matched_proxy") }}</span>
                  <span class="font-mono font-bold text-emerald-600 dark:text-emerald-400">
                    {{ appStatuses[currentTarget.id]?.matched_proxy || "DIRECT" }}
                  </span>
                </div>
              </div>

              <div v-if="appStatuses[currentTarget.id]?.cmdline_preview" class="p-2.5 rounded-lg bg-slate-900 text-slate-300 font-mono text-[11px] overflow-x-auto">
                <div class="text-[10px] text-slate-500 uppercase font-sans mb-1 flex items-center gap-1">
                  <Terminal class="w-3 h-3" />
                  {{ t("process_cmdline") }}
                </div>
                <div class="truncate select-text">{{ appStatuses[currentTarget.id]?.cmdline_preview }}</div>
              </div>
            </div>

            <div v-else class="text-xs text-slate-400 dark:text-slate-500 py-1 flex items-center gap-2">
              <span class="w-1.5 h-1.5 rounded-full bg-slate-300 dark:bg-slate-600"></span>
              <span>{{ t("app_stopped_idle_tip") }}</span>
            </div>
          </div>
        </template>

        <!-- VIEW 3: Empty Fallback -->
        <div v-else class="flex-1 flex flex-col items-center justify-center text-slate-400 space-y-2">
          <Info class="w-8 h-8 text-slate-300 dark:text-slate-600" />
          <p class="text-xs">{{ t("no_app_selected") }}</p>
        </div>
      </main>
    </div>

    <!-- Diagnostics Modal -->
    <div
      v-if="diagModalOpen && activeDiagReport"
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm p-4"
    >
      <div class="bg-white dark:bg-slate-800 rounded-xl shadow-2xl border border-slate-200 dark:border-slate-700 max-w-2xl w-full p-6 space-y-5 animate-in fade-in zoom-in-95 duration-150">
        <div class="flex items-center justify-between border-b border-slate-100 dark:border-slate-700 pb-3">
          <div class="flex items-center gap-2 text-slate-900 dark:text-white font-bold text-base">
            <Activity class="w-5 h-5 text-sky-500" />
            <span>{{ activeDiagReport.app_name }} · {{ t("diag_title") }}</span>
          </div>
          <button @click="diagModalOpen = false" class="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200">
            <X class="w-5 h-5" />
          </button>
        </div>

        <div class="space-y-3 max-h-[60vh] overflow-y-auto pr-1 select-text">
          <div
            v-for="(item, idx) in activeDiagReport.items"
            :key="idx"
            class="p-3.5 rounded-lg border border-slate-200 dark:border-slate-700/80 bg-slate-50 dark:bg-slate-900/40 space-y-1.5 select-text"
          >
            <div class="flex items-center justify-between">
              <span class="text-xs font-bold text-slate-800 dark:text-slate-200 select-text">{{ item.title }}</span>
              <span
                :class="[
                  'text-[10px] font-mono px-2 py-0.5 rounded font-bold uppercase select-none',
                  item.status === 'pass' ? 'bg-emerald-500/20 text-emerald-600 dark:text-emerald-400' : '',
                  item.status === 'warn' ? 'bg-amber-500/20 text-amber-600 dark:text-amber-400' : '',
                  item.status === 'fail' ? 'bg-rose-500/20 text-rose-600 dark:text-rose-400' : '',
                  item.status === 'skipped' ? 'bg-slate-500/20 text-slate-500' : '',
                ]"
              >
                {{ item.status }}
              </span>
            </div>
            <p class="text-xs text-slate-600 dark:text-slate-400 select-text">{{ item.summary }}</p>
            <ul v-if="item.details.length" class="text-[11px] font-mono text-slate-500 dark:text-slate-400 space-y-0.5 mt-1 border-t border-slate-200/50 dark:border-slate-800 pt-1.5 select-text">
              <li v-for="(d, dIdx) in item.details" :key="dIdx" class="select-text">{{ d }}</li>
            </ul>
          </div>
        </div>

        <div class="flex items-center justify-between border-t border-slate-100 dark:border-slate-700 pt-4">
          <button
            @click="copyReport"
            class="px-3.5 py-1.5 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 hover:bg-slate-100 text-slate-700 dark:text-slate-200 flex items-center gap-1.5 transition"
          >
            <Copy class="w-3.5 h-3.5" /> {{ t("diag_copy") }}
          </button>
          <button
            @click="diagModalOpen = false"
            class="px-4 py-1.5 text-xs font-medium rounded-lg bg-sky-600 text-white hover:bg-sky-700 transition"
          >
            {{ t("diag_close") }}
          </button>
        </div>
      </div>
    </div>

    <!-- Add Custom App Modal -->
    <div
      v-if="addModalOpen"
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm p-4"
    >
      <div class="bg-white dark:bg-slate-800 rounded-xl shadow-2xl border border-slate-200 dark:border-slate-700 max-w-md w-full p-6 space-y-4 animate-in fade-in zoom-in-95 duration-150">
        <div class="flex items-center justify-between border-b border-slate-100 dark:border-slate-700 pb-3">
          <h3 class="font-bold text-slate-900 dark:text-white text-base">{{ t("add_modal_title") }}</h3>
          <button @click="addModalOpen = false" class="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200">
            <X class="w-5 h-5" />
          </button>
        </div>

        <div class="space-y-3">
          <div>
            <label class="block text-xs font-medium text-slate-600 dark:text-slate-300 mb-1">{{ t("app_name") }}</label>
            <input
              v-model="customAppForm.name"
              type="text"
              placeholder="e.g. Cursor / Windsurf"
              class="w-full text-xs rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2 focus:ring-2 focus:ring-sky-500 outline-none"
            />
          </div>

          <div>
            <label class="block text-xs font-medium text-slate-600 dark:text-slate-300 mb-1">{{ t("app_exe") }}</label>
            <input
              v-model="customAppForm.exe_path"
              type="text"
              placeholder="C:\Users\...\app.exe"
              class="w-full text-xs font-mono rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2 focus:ring-2 focus:ring-sky-500 outline-none"
            />
          </div>

          <div>
            <label class="block text-xs font-medium text-slate-600 dark:text-slate-300 mb-1">{{ t("app_probe_url") }}</label>
            <input
              v-model="customAppForm.probe_url"
              type="text"
              :placeholder="t('app_probe_placeholder')"
              class="w-full text-xs font-mono rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2 focus:ring-2 focus:ring-sky-500 outline-none"
            />
          </div>

          <div class="flex items-center gap-2 pt-1">
            <input
              id="warnUnsaved"
              v-model="customAppForm.warn_unsaved"
              type="checkbox"
              class="rounded border-slate-300 text-sky-600 focus:ring-sky-500"
            />
            <label for="warnUnsaved" class="text-xs text-slate-600 dark:text-slate-400">{{ t("warn_unsaved") }}</label>
          </div>
        </div>

        <div class="flex items-center justify-end gap-2 border-t border-slate-100 dark:border-slate-700 pt-3">
          <button
            @click="addModalOpen = false"
            class="px-3.5 py-1.5 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 text-slate-600 dark:text-slate-300 hover:bg-slate-50"
          >
            {{ t("btn_cancel") }}
          </button>
          <button
            @click="handleAddCustomApp"
            class="px-4 py-1.5 text-xs font-medium rounded-lg bg-sky-600 text-white hover:bg-sky-700"
          >
            {{ t("btn_confirm") }}
          </button>
        </div>
      </div>
    </div>

    <!-- Confirm Stop Modal for warn_unsaved_on_kill -->
    <div
      v-if="confirmStopModal.open && confirmStopModal.target"
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm p-4"
    >
      <div class="bg-white dark:bg-slate-800 rounded-xl shadow-2xl border border-amber-500/30 dark:border-amber-500/30 max-w-md w-full p-6 space-y-4 animate-in fade-in zoom-in-95 duration-150">
        <div class="flex items-center gap-2.5 text-amber-600 dark:text-amber-400 font-bold text-base">
          <AlertTriangle class="w-5 h-5" />
          <h3>{{ t("warn_stop_title") }}</h3>
        </div>
        <p class="text-xs text-slate-600 dark:text-slate-300 leading-relaxed">
          {{ currentLang === 'en-US' && confirmStopModal.target.name_en ? confirmStopModal.target.name_en : confirmStopModal.target.name }}：
          {{ t("warn_stop_desc") }}
        </p>
        <div class="flex items-center justify-end gap-2 border-t border-slate-100 dark:border-slate-700 pt-3">
          <button
            @click="confirmStopModal.open = false"
            class="px-3.5 py-1.5 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 text-slate-600 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-700/50"
          >
            {{ t("btn_cancel") }}
          </button>
          <button
            @click="confirmStopModal.relaunchAfterStop ? executeLaunch(confirmStopModal.target, false) : executeStop(confirmStopModal.target, false)"
            class="px-3.5 py-1.5 text-xs font-medium rounded-lg bg-sky-600 text-white hover:bg-sky-700 shadow-sm"
          >
            {{ t("btn_confirm_close") }}
          </button>
          <button
            @click="confirmStopModal.relaunchAfterStop ? executeLaunch(confirmStopModal.target, true) : executeStop(confirmStopModal.target, true)"
            class="px-3.5 py-1.5 text-xs font-medium rounded-lg bg-rose-600 text-white hover:bg-rose-700 shadow-sm"
          >
            {{ t("btn_force_kill") }}
          </button>
        </div>
      </div>
    </div>

    <!-- Confirm Remove Custom App Modal -->
    <div
      v-if="confirmRemoveModal.open"
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm p-4"
    >
      <div class="bg-white dark:bg-slate-800 rounded-xl shadow-2xl border border-rose-500/30 max-w-md w-full p-6 space-y-4 animate-in fade-in zoom-in-95 duration-150">
        <div class="flex items-center gap-2.5 text-rose-600 dark:text-rose-400 font-bold text-base">
          <Trash2 class="w-5 h-5" />
          <h3>{{ t("confirm_remove_title") }}</h3>
        </div>
        <p class="text-xs text-slate-600 dark:text-slate-300 leading-relaxed">
          {{ confirmRemoveModal.appName }}：{{ t("confirm_remove_desc") }}
        </p>
        <div
          v-if="confirmRemoveModal.isRunning"
          class="p-2.5 rounded-lg bg-amber-500/10 border border-amber-500/30 text-amber-600 dark:text-amber-400 text-xs flex items-center gap-2"
        >
          <AlertTriangle class="w-4 h-4 shrink-0" />
          <span>{{ t("confirm_remove_running_note") }}</span>
        </div>
        <div class="flex items-center justify-end gap-2 border-t border-slate-100 dark:border-slate-700 pt-3">
          <button
            @click="confirmRemoveModal.open = false"
            class="px-3.5 py-1.5 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 text-slate-600 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-700/50"
          >
            {{ t("btn_cancel") }}
          </button>
          <button
            @click="executeRemoveCustomApp"
            class="px-4 py-1.5 text-xs font-medium rounded-lg bg-rose-600 text-white hover:bg-rose-700 shadow-sm"
          >
            {{ t("btn_remove_app") }}
          </button>
        </div>
      </div>
    </div>

    <!-- Confirm Switch Account Modal -->
    <div
      v-if="confirmSwitchModal.open"
      class="fixed inset-0 z-50 bg-black/50 backdrop-blur-xs flex items-center justify-center p-4 animate-in fade-in"
    >
      <div class="bg-white dark:bg-slate-800 rounded-xl max-w-sm w-full p-5 space-y-4 shadow-xl border border-slate-200 dark:border-slate-700">
        <div class="flex items-center gap-3">
          <div class="w-10 h-10 rounded-full bg-sky-100 dark:bg-sky-900/60 flex items-center justify-center text-sky-600 shrink-0">
            <ArrowRightLeft class="w-5 h-5" />
          </div>
          <div>
            <h3 class="text-sm font-bold text-slate-900 dark:text-white">{{ t("confirm_switch_title") }}</h3>
            <p class="text-xs text-sky-600 dark:text-sky-400 font-semibold mt-0.5">{{ confirmSwitchModal.accountName }}</p>
          </div>
        </div>

        <p class="text-xs text-slate-500 dark:text-slate-400 leading-relaxed">
          {{ t("confirm_switch_desc") }}
        </p>

        <div class="flex items-center justify-end gap-2 pt-2 border-t border-slate-100 dark:border-slate-700/60">
          <button
            @click="confirmSwitchModal.open = false"
            class="px-3.5 py-1.5 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 hover:bg-slate-50 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-300 transition"
          >
            {{ t("btn_cancel") }}
          </button>
          <button
            @click="executeSwitchAccount"
            :disabled="isSwitchingAccount"
            class="px-4 py-1.5 text-xs font-semibold rounded-lg bg-sky-600 hover:bg-sky-700 text-white flex items-center gap-1 shadow-sm transition disabled:opacity-50"
          >
            <RotateCw v-if="isSwitchingAccount" class="w-3.5 h-3.5 animate-spin" />
            <span>{{ t("btn_confirm") }}</span>
          </button>
        </div>
      </div>
    </div>

    <!-- Add / Import Account Modal -->
    <div
      v-if="addAccountModal.open"
      class="fixed inset-0 z-50 bg-black/50 backdrop-blur-xs flex items-center justify-center p-4 animate-in fade-in"
    >
      <div class="bg-white dark:bg-slate-800 rounded-xl max-w-lg w-full p-5 space-y-4 shadow-xl border border-slate-200 dark:border-slate-700">
        <div class="flex items-center justify-between border-b border-slate-100 dark:border-slate-700/60 pb-3">
          <div class="flex items-center gap-2">
            <PlusCircle class="w-4 h-4 text-sky-500" />
            <h3 class="text-sm font-bold text-slate-900 dark:text-white">{{ t("modal_add_account_title") }}</h3>
          </div>
          <button @click="addAccountModal.open = false" class="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200">
            <X class="w-4 h-4" />
          </button>
        </div>

        <div class="space-y-3">
          <div>
            <label class="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
              {{ t("account_custom_name") }}
            </label>
            <input
              v-model="addAccountModal.name"
              type="text"
              placeholder="e.g. Work, Personal, Client-1"
              class="w-full text-xs rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2.5 focus:ring-2 focus:ring-sky-500 outline-none"
            />
          </div>

          <div>
            <label class="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
              {{ t("account_credential_content") }}
            </label>
            <textarea
              v-model="addAccountModal.content"
              rows="6"
              :placeholder="t('account_content_placeholder')"
              class="w-full text-xs font-mono rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2.5 focus:ring-2 focus:ring-sky-500 outline-none"
            ></textarea>
          </div>
        </div>

        <div class="flex items-center justify-end gap-2 pt-2 border-t border-slate-100 dark:border-slate-700/60">
          <button
            @click="addAccountModal.open = false"
            class="px-3.5 py-1.5 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 hover:bg-slate-50 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-300 transition"
          >
            {{ t("btn_cancel") }}
          </button>
          <button
            @click="handleAddAccountSubmit"
            class="px-4 py-1.5 text-xs font-semibold rounded-lg bg-sky-600 hover:bg-sky-700 text-white shadow-sm transition"
          >
            {{ t("btn_confirm") }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
