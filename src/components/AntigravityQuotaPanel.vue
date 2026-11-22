<script setup lang="ts">
import { AlertTriangle, Clock, Copy, ExternalLink } from "lucide-vue-next";
import { t } from "../i18n";
import {
  getAntigravityQuotaClass,
  type AntigravityQuotaView,
} from "../antigravityQuota";

defineProps<{
  view: AntigravityQuotaView;
  formatCountdown: (sec?: number) => string;
}>();

const emit = defineEmits<{
  (e: "open-validation-url", url: string): void;
  (e: "copy-validation-url", url: string): void;
}>();
</script>

<template>
  <!-- Warning / Reauth / Error Banner -->
  <div
    v-if="view.isStale || view.isNeedsReauth || view.isForbidden || view.errorMessage"
    :class="[
      'p-3 rounded-lg border text-xs flex flex-wrap items-center justify-between gap-2',
      view.isForbidden && !view.buckets.length
        ? 'border-rose-300 dark:border-rose-800/70 bg-rose-50/70 dark:bg-rose-950/30 text-rose-700 dark:text-rose-300'
        : 'border-amber-300 dark:border-amber-800/70 bg-amber-50/70 dark:bg-amber-950/30 text-amber-700 dark:text-amber-300'
    ]"
  >
    <div class="flex items-center gap-2 min-w-0">
      <AlertTriangle class="w-4 h-4 shrink-0" />
      <span class="leading-snug break-words">
        <template v-if="view.isStale && view.isNeedsReauth && view.buckets.length">
          {{ t("quota_cached_needs_reauth") }}
        </template>
        <template v-else-if="view.isStale && view.buckets.length">
          {{ t("quota_cached_refresh_failed") }}
        </template>
        <template v-else-if="view.errorMessage">
          {{ view.errorMessage }}
        </template>
        <template v-else>
          {{ t("quota_cached_refresh_failed") }}
        </template>
      </span>
    </div>
    <div v-if="view.validationUrl" class="flex items-center gap-1.5 shrink-0">
      <button
        @click="emit('open-validation-url', view.validationUrl)"
        class="px-2.5 py-1 rounded bg-amber-600 hover:bg-amber-700 text-white text-[11px] font-semibold flex items-center gap-1 transition"
      >
        <ExternalLink class="w-3 h-3" />
        <span>{{ t("quota_open_validation_url") }}</span>
      </button>
      <button
        @click="emit('copy-validation-url', view.validationUrl)"
        class="px-2 py-1 rounded border border-amber-300 dark:border-amber-700 hover:bg-amber-100 dark:hover:bg-amber-900/50 text-[11px] font-medium flex items-center gap-1 transition"
      >
        <Copy class="w-3 h-3" />
        <span>{{ t("quota_copy_validation_url") }}</span>
      </button>
    </div>
  </div>

  <!-- Summary Buckets Grid -->
  <div v-if="view.buckets.length" class="grid grid-cols-1 md:grid-cols-2 gap-3">
    <div
      v-for="b in view.buckets"
      :key="b.key"
      class="p-3.5 rounded-lg border border-slate-200/80 dark:border-slate-700/80 bg-white dark:bg-slate-800/80 flex flex-col justify-between space-y-2.5 shadow-2xs hover:border-slate-300 dark:hover:border-slate-600 transition"
    >
      <div class="flex flex-wrap items-baseline justify-between gap-x-2 gap-y-1">
        <span class="text-xs font-bold text-slate-800 dark:text-slate-100 break-words leading-snug">
          {{ b.label }}
        </span>
        <span
          v-if="b.blockedByWeekly"
          class="text-xs font-bold font-mono text-rose-500 shrink-0 ml-auto"
          :title="t('quota_weekly_limit_reached')"
        >
          —
        </span>
        <span
          v-else-if="!b.hasData || b.remainingPercent == null"
          class="text-xs font-bold font-mono text-slate-400 shrink-0 ml-auto"
          :title="t('quota_no_data')"
        >
          —
        </span>
        <span v-else class="text-xs font-bold font-mono shrink-0 whitespace-nowrap ml-auto">
          <span class="text-amber-600 dark:text-amber-400">{{ b.usedPercent }}% {{ t("quota_used") }}</span>
          <span class="text-slate-300 dark:text-slate-600 mx-1">/</span>
          <span class="text-sky-600 dark:text-sky-400">{{ b.remainingPercent }}% {{ t("quota_remaining") }}</span>
        </span>
      </div>

      <!-- Progress Bar -->
      <div class="w-full bg-slate-100 dark:bg-slate-700 h-2 rounded-full overflow-hidden">
        <div
          v-if="b.hasData && b.remainingPercent != null"
          class="h-full rounded-full transition-all duration-300"
          :class="[
            getAntigravityQuotaClass(b.remainingPercent, b.hasData) === 'high'
              ? 'bg-emerald-500'
              : getAntigravityQuotaClass(b.remainingPercent, b.hasData) === 'medium'
              ? 'bg-amber-500'
              : 'bg-rose-500'
          ]"
          :style="{ width: `${Math.max(0, Math.min(100, b.remainingPercent))}%` }"
        ></div>
      </div>

      <!-- Reset / Status Line -->
      <div class="flex items-center justify-between text-[11px] text-slate-400 pt-1.5 border-t border-slate-100 dark:border-slate-700/50">
        <span v-if="b.blockedByWeekly" class="text-rose-500 font-medium">
          {{ t("quota_weekly_limit_reached") }}
        </span>
        <span v-else-if="!b.hasData" class="text-slate-400">
          {{ t("quota_no_data") }}
        </span>
        <template v-else>
          <div class="flex items-center gap-1.5 text-slate-500 dark:text-slate-400" :title="b.resetsAt || undefined">
            <Clock class="w-3.5 h-3.5 text-slate-400 shrink-0" />
            <span v-if="b.resetInSeconds && b.resetInSeconds > 0" class="font-mono text-sky-600 dark:text-sky-400 font-semibold">
              {{ formatCountdown(b.resetInSeconds) }}
            </span>
            <span v-else>--</span>
          </div>
          <span v-if="b.resetsAt" class="font-mono text-[10px] text-slate-400 shrink-0 ml-auto whitespace-nowrap">
            {{ b.resetsAt }}
          </span>
        </template>
      </div>
    </div>
  </div>

  <div
    v-else-if="!view.models.length && !view.errorMessage"
    class="p-4 rounded-lg border border-dashed border-slate-200 dark:border-slate-700 text-center text-xs text-slate-400"
  >
    {{ t("quota_no_data") }}
  </div>

  <!-- Collapsible Per-Model Quotas -->
  <details
    v-if="view.models.length"
    :open="view.buckets.length === 0"
    class="rounded-lg border border-slate-200/80 dark:border-slate-700/80 bg-white/70 dark:bg-slate-800/50 p-3"
  >
    <summary class="text-xs font-semibold text-slate-700 dark:text-slate-200 cursor-pointer select-none flex items-center justify-between">
      <span>{{ t("quota_model_details_title") }} ({{ view.models.length }})</span>
    </summary>
    <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-2.5 mt-3">
      <div
        v-for="m in view.models"
        :key="m.key"
        class="p-2.5 rounded-lg border border-slate-200/70 dark:border-slate-700/70 bg-white dark:bg-slate-800/80 space-y-1.5"
      >
        <div class="flex items-baseline justify-between gap-2">
          <span class="text-[11px] font-semibold text-slate-700 dark:text-slate-200 truncate" :title="m.label">{{ m.label }}</span>
          <span
            class="text-[11px] font-bold font-mono shrink-0"
            :class="[
              getAntigravityQuotaClass(m.remainingPercent, m.remainingPercent != null) === 'high' ? 'text-emerald-600 dark:text-emerald-400' :
              getAntigravityQuotaClass(m.remainingPercent, m.remainingPercent != null) === 'medium' ? 'text-amber-600 dark:text-amber-400' :
              'text-rose-600 dark:text-rose-400'
            ]"
          >
            {{ m.remainingPercent != null ? `${m.remainingPercent}%` : "—" }}
          </span>
        </div>
        <div class="w-full bg-slate-100 dark:bg-slate-700 h-1.5 rounded-full overflow-hidden">
          <div
            v-if="m.remainingPercent != null"
            class="h-full rounded-full transition-all duration-300"
            :class="[
              getAntigravityQuotaClass(m.remainingPercent, true) === 'high' ? 'bg-emerald-500' :
              getAntigravityQuotaClass(m.remainingPercent, true) === 'medium' ? 'bg-amber-500' :
              'bg-rose-500'
            ]"
            :style="{ width: `${Math.max(0, Math.min(100, m.remainingPercent))}%` }"
          ></div>
        </div>
        <div class="flex items-center justify-between text-[10px] text-slate-400 font-mono">
          <span>{{ m.resetInSeconds && m.resetInSeconds > 0 ? formatCountdown(m.resetInSeconds) : "--" }}</span>
          <span v-if="m.resetsAt">{{ m.resetsAt }}</span>
        </div>
      </div>
    </div>
  </details>
</template>
