<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  AppSettings,
  AppRuntimeState,
  DiagReport,
  TargetAppSpec,
  RunStatus,
} from "./types";
import { t, currentLang } from "./i18n";
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
const showNoProxy = ref(false);
const scannedPorts = ref<number[]>([]);
const isScanning = ref(false);
const showPortDropdown = ref(false);
const saveSuccess = ref(false);

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
}>({
  open: false,
  target: null,
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
    }
    applyTheme(res.theme || "system");
    await refreshTargets();
    await refreshStatuses();
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

const allTargets = computed<TargetAppSpec[]>(() => effectiveTargets.value);

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
      showToast("已自动检测并填入路径", "success");
    } else {
      showToast(t("toast_auto_detect_failed"), "warn");
    }
  } catch (err) {
    showToast(`Auto-detect error: ${err}`, "error");
  }
}

async function handleLaunch(target: TargetAppSpec) {
  try {
    await invoke("launch_app", { appId: target.id });
    await refreshStatuses();
  } catch (err) {
    showToast(`Launch failed: ${err}`, "error");
  }
}

function handleStop(target: TargetAppSpec) {
  if (target.warn_unsaved_on_kill) {
    confirmStopModal.value = { open: true, target };
  } else {
    executeStop(target, false);
  }
}

async function executeStop(target: TargetAppSpec, force = false) {
  try {
    await invoke("stop_app", { appId: target.id, force });
    await refreshStatuses();
    showToast(force ? "已强制结束进程" : "已请求安全退出", "info");
  } catch (err) {
    showToast(`Stop failed: ${err}`, "error");
  } finally {
    confirmStopModal.value = { open: false, target: null };
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

onMounted(() => {
  loadInitialData();
  statusPollTimer = window.setInterval(refreshStatuses, 2500);
});

onUnmounted(() => {
  if (statusPollTimer) clearInterval(statusPollTimer);
});
</script>

<template>
  <div class="min-h-screen flex flex-col p-5 max-w-5xl mx-auto space-y-5">
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
    <header class="flex items-center justify-between bg-white dark:bg-slate-800 p-4 rounded-xl shadow-sm border border-slate-200/80 dark:border-slate-700/80">
      <div class="flex items-center gap-3">
        <div class="w-10 h-10 rounded-xl bg-gradient-to-tr from-sky-500 to-indigo-600 flex items-center justify-center text-white shadow-md shadow-sky-500/20">
          <Zap class="w-6 h-6" />
        </div>
        <div>
          <h1 class="text-xl font-bold tracking-tight text-slate-900 dark:text-white flex items-center gap-2">
            {{ t("app_title") }}
            <span class="text-xs px-2 py-0.5 rounded-full bg-sky-100 text-sky-700 dark:bg-sky-900/50 dark:text-sky-300 font-mono">v0.2.0 (Tauri)</span>
          </h1>
          <p class="text-xs text-slate-500 dark:text-slate-400">{{ t("app_subtitle") }}</p>
        </div>
      </div>

      <div class="flex items-center gap-3">
        <!-- Autostart Toggle -->
        <label class="flex items-center gap-1.5 text-xs text-slate-600 dark:text-slate-300 cursor-pointer select-none px-2.5 py-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 hover:bg-slate-100 dark:hover:bg-slate-700 transition">
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
          class="px-2.5 py-1.5 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 hover:bg-slate-100 dark:hover:bg-slate-700 transition"
        >
          <Globe class="w-3.5 h-3.5 inline mr-1" />
          {{ currentLang === 'zh-CN' ? 'English' : '简体中文' }}
        </button>

        <!-- Theme Switch -->
        <div class="flex items-center rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 p-0.5">
          <button
            @click="applyTheme('light'); saveCurrentSettings();"
            :class="['p-1.5 rounded-md text-xs transition', settings.theme === 'light' ? 'bg-white dark:bg-slate-600 shadow-sm text-amber-500' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200']"
            title="Light"
          >
            <Sun class="w-3.5 h-3.5" />
          </button>
          <button
            @click="applyTheme('dark'); saveCurrentSettings();"
            :class="['p-1.5 rounded-md text-xs transition', settings.theme === 'dark' ? 'bg-white dark:bg-slate-600 shadow-sm text-sky-400' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200']"
            title="Dark"
          >
            <Moon class="w-3.5 h-3.5" />
          </button>
          <button
            @click="applyTheme('system'); saveCurrentSettings();"
            :class="['p-1.5 rounded-md text-xs transition', settings.theme === 'system' ? 'bg-white dark:bg-slate-600 shadow-sm text-indigo-500' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200']"
            title="System"
          >
            <Monitor class="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </header>

    <!-- Global Proxy Card -->
    <section class="bg-white dark:bg-slate-800 p-5 rounded-xl shadow-sm border border-slate-200/80 dark:border-slate-700/80 space-y-4">
      <div class="flex items-center justify-between border-b border-slate-100 dark:border-slate-700/50 pb-3">
        <div class="flex items-center gap-2 text-slate-900 dark:text-white font-semibold">
          <ShieldCheck class="w-5 h-5 text-sky-500" />
          <span>{{ t("global_proxy_title") }}</span>
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

      <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
        <!-- Scheme -->
        <div>
          <label class="block text-xs font-medium text-slate-600 dark:text-slate-300 mb-1.5">{{ t("proxy_scheme") }}</label>
          <select
            v-model="settings.proxy.scheme"
            class="w-full text-sm rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2 focus:ring-2 focus:ring-sky-500 outline-none"
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
            class="w-full text-sm rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2 focus:ring-2 focus:ring-sky-500 outline-none"
          />
        </div>

        <!-- Port with Scanner -->
        <div class="relative">
          <div class="flex items-center justify-between mb-1.5">
            <label class="text-xs font-medium text-slate-600 dark:text-slate-300">{{ t("proxy_port") }}</label>
            <button
              @click="handleScanPorts"
              class="text-xs text-sky-600 dark:text-sky-400 hover:underline flex items-center gap-1 font-medium"
            >
              <Search class="w-3 h-3" />
              {{ isScanning ? '扫描中...' : t("scan_ports") }}
            </button>
          </div>
          <input
            v-model.number="settings.proxy.port"
            type="number"
            placeholder="7890"
            class="w-full text-sm rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2 focus:ring-2 focus:ring-sky-500 outline-none"
          />

          <!-- Scanned Ports Dropdown -->
          <div
            v-if="showPortDropdown && scannedPorts.length > 0"
            class="absolute z-20 left-0 right-0 mt-1 bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-lg shadow-lg overflow-hidden py-1"
          >
            <div class="text-xs px-3 py-1 font-semibold text-slate-400 border-b border-slate-100 dark:border-slate-700">检测到的本地监听端口:</div>
            <button
              v-for="p in scannedPorts"
              :key="p"
              @click="selectScannedPort(p)"
              class="w-full text-left px-3 py-1.5 text-xs text-slate-700 dark:text-slate-200 hover:bg-sky-50 dark:hover:bg-slate-700/60 flex items-center justify-between"
            >
              <span class="font-mono font-medium">{{ p }}</span>
              <span class="text-emerald-500 text-[10px]">在线</span>
            </button>
          </div>
        </div>
      </div>

      <!-- NO_PROXY Bypass Collapsible -->
      <div class="border-t border-slate-100 dark:border-slate-700/40 pt-3">
        <button
          @click="showNoProxy = !showNoProxy"
          class="flex items-center gap-1.5 text-xs text-slate-500 dark:text-slate-400 hover:text-slate-700 dark:hover:text-slate-200 transition font-medium"
        >
          <component :is="showNoProxy ? ChevronUp : ChevronDown" class="w-3.5 h-3.5" />
          {{ t("no_proxy_bypass") }}
        </button>

        <div v-if="showNoProxy" class="mt-2">
          <textarea
            v-model="settings.proxy.no_proxy"
            rows="2"
            class="w-full text-xs font-mono rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2.5 focus:ring-2 focus:ring-sky-500 outline-none"
          ></textarea>
        </div>
      </div>
    </section>

    <!-- Target Apps Grid -->
    <section class="space-y-4">
      <div class="flex items-center justify-between px-1">
        <h2 class="text-sm font-bold text-slate-700 dark:text-slate-200 tracking-wide uppercase">AI 应用列表 (Target Apps)</h2>
        <button
          @click="addModalOpen = true"
          class="text-xs font-medium text-sky-600 dark:text-sky-400 hover:text-sky-700 flex items-center gap-1"
        >
          <Plus class="w-3.5 h-3.5" /> {{ t("add_custom_app") }}
        </button>
      </div>

      <div class="grid grid-cols-1 gap-4">
        <div
          v-for="target in allTargets"
          :key="target.id"
          class="bg-white dark:bg-slate-800 p-5 rounded-xl shadow-sm border border-slate-200/80 dark:border-slate-700/80 flex flex-col justify-between space-y-4 transition hover:border-slate-300 dark:hover:border-slate-600"
        >
          <!-- Card Top: Name & Badges -->
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-2.5">
              <h3 class="font-bold text-slate-900 dark:text-white text-base">
                {{ currentLang === 'en-US' && target.name_en ? target.name_en : target.name }}
              </h3>
              <span
                :class="['text-xs px-2.5 py-0.5 rounded-full border font-medium flex items-center gap-1.5', statusColor(appStatuses[target.id]?.status)]"
              >
                <span class="w-1.5 h-1.5 rounded-full bg-current animate-pulse"></span>
                {{ statusText(appStatuses[target.id]?.status) }}
              </span>
            </div>

            <!-- Mode Switch (Proxied vs Direct) & Actions -->
            <div class="flex items-center gap-2">
              <div class="flex items-center bg-slate-100 dark:bg-slate-900/70 p-0.5 rounded-lg border border-slate-200 dark:border-slate-700">
                <button
                  @click="!target.proxy_enabled && toggleProxyMode(target)"
                  :class="['px-2.5 py-1 text-xs font-medium rounded-md transition', target.proxy_enabled ? 'bg-sky-600 text-white shadow-sm' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200']"
                >
                  {{ t("mode_proxied") }}
                </button>
                <button
                  @click="target.proxy_enabled && toggleProxyMode(target)"
                  :class="['px-2.5 py-1 text-xs font-medium rounded-md transition', !target.proxy_enabled ? 'bg-slate-600 text-white shadow-sm' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200']"
                >
                  {{ t("mode_direct") }}
                </button>
              </div>

              <!-- Delete Custom App Button -->
              <button
                v-if="!target.builtin"
                @click="handlePromptRemoveCustomApp(target)"
                class="p-1.5 rounded-lg text-slate-400 hover:text-rose-600 hover:bg-rose-50 dark:hover:bg-rose-950/40 transition"
                :title="t('btn_remove_app')"
              >
                <Trash2 class="w-4 h-4" />
              </button>
            </div>
          </div>

          <!-- Card Middle: Exe Path Input & Auto-detect -->
          <div>
            <div class="flex items-center gap-2">
              <input
                v-model="target.exe_path"
                type="text"
                :placeholder="t('path_not_found')"
                class="flex-1 text-xs font-mono rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-900/60 p-2 focus:ring-2 focus:ring-sky-500 outline-none text-slate-600 dark:text-slate-300"
                @change="onTargetExeChange(target)"
              />
              <button
                v-if="target.builtin"
                @click="handleAutoDetect(target.id)"
                class="px-2.5 py-2 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-600 dark:text-slate-300 flex items-center gap-1 transition whitespace-nowrap"
              >
                <Search class="w-3.5 h-3.5" /> {{ t("btn_detect") }}
              </button>
            </div>
          </div>

          <!-- Card Bottom: Actions -->
          <div class="flex items-center justify-between border-t border-slate-100 dark:border-slate-700/40 pt-3">
            <div class="flex items-center gap-2">
              <!-- Launch / Restart Button -->
              <button
                v-if="appStatuses[target.id]?.status === 'restart_needed'"
                @click="handleLaunch(target)"
                class="px-4 py-1.5 text-xs font-medium rounded-lg bg-rose-600 text-white hover:bg-rose-700 flex items-center gap-1.5 shadow-sm transition"
              >
                <RotateCw class="w-3.5 h-3.5" /> {{ t("btn_restart") }}
              </button>
              <button
                v-else
                @click="handleLaunch(target)"
                class="px-4 py-1.5 text-xs font-medium rounded-lg bg-emerald-600 text-white hover:bg-emerald-700 flex items-center gap-1.5 shadow-sm transition"
              >
                <Play class="w-3.5 h-3.5" /> {{ t("btn_launch") }}
              </button>

              <!-- Stop Button -->
              <button
                v-if="appStatuses[target.id]?.status !== 'stopped'"
                @click="handleStop(target)"
                class="px-3 py-1.5 text-xs font-medium rounded-lg border border-rose-300 dark:border-rose-800/80 text-rose-600 dark:text-rose-400 hover:bg-rose-50 dark:hover:bg-rose-950/40 flex items-center gap-1.5 transition"
              >
                <Square class="w-3.5 h-3.5" /> {{ t("btn_stop") }}
              </button>
            </div>

            <div class="flex items-center gap-2">
              <!-- Diagnostics Button -->
              <button
                @click="handleRunDiagnostics(target)"
                class="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 flex items-center gap-1.5 transition"
              >
                <Activity class="w-3.5 h-3.5 text-sky-500" /> {{ t("btn_diagnostics") }}
              </button>

              <!-- Shortcut Button -->
              <button
                @click="handleCreateShortcut(target)"
                class="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-700/50 hover:bg-slate-100 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 flex items-center gap-1.5 transition"
                title="在桌面生成带代理参数的快捷方式"
              >
                <ExternalLink class="w-3.5 h-3.5 text-indigo-500" /> {{ t("btn_shortcut") }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </section>

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
              placeholder="e.g. Claude Desktop / Cursor"
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
            @click="executeStop(confirmStopModal.target, false)"
            class="px-3.5 py-1.5 text-xs font-medium rounded-lg bg-sky-600 text-white hover:bg-sky-700 shadow-sm"
          >
            {{ t("btn_confirm_close") }}
          </button>
          <button
            @click="executeStop(confirmStopModal.target, true)"
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
  </div>
</template>
