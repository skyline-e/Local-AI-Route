import type { AccountProfile, QuotaWindow } from "./types";

export type QuotaHealthClass = "high" | "medium" | "low" | "neutral";

export interface AntigravityBucketSlot {
  key: string;
  label: string;
  remainingPercent: number | null;
  usedPercent: number | null;
  resetsAt?: string;
  resetInSeconds?: number;
  detail?: string;
  stale: boolean;
  blockedByWeekly: boolean;
  hasData: boolean;
}

export interface AntigravityModelQuotaItem {
  key: string;
  label: string;
  remainingPercent: number | null;
  usedPercent: number | null;
  resetsAt?: string;
  resetInSeconds?: number;
  detail?: string;
}

export interface AntigravityQuotaView {
  buckets: AntigravityBucketSlot[];
  models: AntigravityModelQuotaItem[];
  isStale: boolean;
  isNeedsReauth: boolean;
  isForbidden: boolean;
  errorMessage?: string;
  validationUrl?: string;
  aiCredits?: string;
}

export interface AntigravityMiniPill {
  key: string;
  shortLabel: string;
  percentageText: string;
  quotaClass: QuotaHealthClass;
  blockedByWeekly: boolean;
  stale: boolean;
}

const PAID_BUCKET_KEYS = ["gemini:5h", "gemini:weekly", "claude:5h", "claude:weekly"] as const;
const FREE_BUCKET_KEYS = ["gemini:weekly", "claude:weekly"] as const;
const STANDARD_BUCKET_SET = new Set<string>(PAID_BUCKET_KEYS);

const BUCKET_LABELS: Record<string, { zh: string; en: string; short: string }> = {
  "gemini:5h": { zh: "Gemini · 5小时", en: "Gemini · 5h", short: "G·5h" },
  "gemini:weekly": { zh: "Gemini · 周用量", en: "Gemini · Weekly", short: "G·W" },
  "claude:5h": { zh: "Claude · 5小时", en: "Claude · 5h", short: "C·5h" },
  "claude:weekly": { zh: "Claude · 周用量", en: "Claude · Weekly", short: "C·W" },
  "argon:weekly": { zh: "Gemini 4 Argon · 周用量", en: "Gemini 4 Argon · Weekly", short: "Argon" },
  "carbon:weekly": { zh: "Gemini 4 Carbon · 周用量", en: "Gemini 4 Carbon · Weekly", short: "Carbon" },
};

export function getAntigravityQuotaClass(
  remainingPercent: number | null | undefined,
  hasData = true,
): QuotaHealthClass {
  if (!hasData || remainingPercent == null || !Number.isFinite(remainingPercent)) {
    return "neutral";
  }
  if (remainingPercent >= 70) return "high";
  if (remainingPercent >= 30) return "medium";
  return "low";
}

function resolveBucketLabel(key: string, fallbackName: string, lang: string): string {
  const entry = BUCKET_LABELS[key];
  if (!entry) return fallbackName || key;
  return lang === "en-US" ? entry.en : entry.zh;
}

function resolveBucketShortLabel(key: string, fallbackName: string): string {
  return BUCKET_LABELS[key]?.short || fallbackName || key;
}

function inferCanonicalBucketKey(w: QuotaWindow): string | null {
  const wk = w.window_key?.trim().toLowerCase();
  if (wk === "3p-5h" || wk === "claude:5h") return "claude:5h";
  if (wk === "3p-weekly" || wk === "claude:weekly") return "claude:weekly";
  if (wk === "gemini-5h" || wk === "gemini:5h") return "gemini:5h";
  if (wk === "gemini-weekly" || wk === "gemini:weekly") return "gemini:weekly";
  if (wk === "argon:weekly" || wk === "gemini-argon-wtu-weekly") return "argon:weekly";
  if (wk === "carbon:weekly" || wk === "gemini-carbon-wtu-weekly") return "carbon:weekly";
  if (wk && wk.startsWith("gemini-argon-") && wk.endsWith("-weekly")) return "argon:weekly";
  if (wk && wk.startsWith("gemini-carbon-") && wk.endsWith("-weekly")) return "carbon:weekly";
  if (w.window_kind === "bucket" && wk) return wk;

  if (w.window_kind === "model") return null;

  // Legacy saved windows before window_kind/window_key existed
  const lower = w.name.toLowerCase();
  if (lower.includes("argon") && w.name.includes("周")) return "argon:weekly";
  if (lower.includes("carbon") && w.name.includes("周")) return "carbon:weekly";
  if (lower.includes("gemini") && w.name.includes("周")) return "gemini:weekly";
  if (lower.includes("gemini") && w.name.includes("5")) return "gemini:5h";
  if ((lower.includes("claude") || lower.includes("gpt")) && w.name.includes("周")) {
    return "claude:weekly";
  }
  if ((lower.includes("claude") || lower.includes("gpt")) && w.name.includes("5")) {
    return "claude:5h";
  }
  return null;
}

function resolveSubscriptionTier(account: AccountProfile): "ULTRA" | "PRO" | "FREE" | "UNKNOWN" {
  const raw =
    account.quota?.extra_info?.["subscription_tier"] ||
    account.quota?.extra_info?.["tier_id"] ||
    account.quota?.plan_type ||
    account.plan_type ||
    "";
  const lower = raw.trim().toLowerCase();
  if (!lower) return "UNKNOWN";
  if (lower.includes("ultra")) return "ULTRA";
  if (lower.includes("pro")) return "PRO";
  if (lower.includes("free") || lower.includes("standard") || lower.includes("legacy")) return "FREE";
  return "FREE";
}

export function isAntigravityAccountNeedsReauth(account: AccountProfile): boolean {
  const extra = account.quota?.extra_info;
  if (!extra) return false;
  const reason = extra["quota_error_reason"] || "";
  if (reason === "VALIDATION_REQUIRED" || reason.startsWith("invalid_grant")) {
    return true;
  }
  if (extra["validation_url"]) {
    return true;
  }
  const msg = extra["quota_error_message"] || "";
  return (
    msg.includes("Verify your account") ||
    msg.includes("VALIDATION_REQUIRED") ||
    msg.includes("invalid_grant")
  );
}

export function getAntigravityQuotaView(account: AccountProfile, lang: string): AntigravityQuotaView {
  const quota = account.quota;
  const extra = quota?.extra_info || {};
  const windows = quota?.windows || [];
  const tier = resolveSubscriptionTier(account);
  const isFree = tier === "FREE";

  const bucketMap = new Map<string, QuotaWindow>();
  const extraBuckets: Array<{ key: string; win: QuotaWindow }> = [];
  const models: AntigravityModelQuotaItem[] = [];

  for (const w of windows) {
    if (w.window_kind === "model") {
      models.push({
        key: w.window_key || w.name,
        label: w.name,
        remainingPercent: w.remaining_percent ?? null,
        usedPercent: w.used_percent ?? null,
        resetsAt: w.resets_at,
        resetInSeconds: w.reset_in_seconds,
        detail: w.detail,
      });
      continue;
    }

    const bucketKey = inferCanonicalBucketKey(w);
    if (bucketKey) {
      if (isFree && bucketKey.endsWith(":5h")) {
        continue;
      }
      if (!bucketMap.has(bucketKey)) {
        bucketMap.set(bucketKey, w);
        if (!STANDARD_BUCKET_SET.has(bucketKey)) {
          extraBuckets.push({ key: bucketKey, win: w });
        }
      }
    } else if (w.window_kind === "bucket") {
      const k = w.window_key || w.name;
      if (!bucketMap.has(k)) {
        bucketMap.set(k, w);
        extraBuckets.push({ key: k, win: w });
      }
    }
  }

  const hasAny5h = Array.from(bucketMap.keys()).some((k) => k.endsWith(":5h"));
  const hasBuckets = bucketMap.size > 0;
  const usePaidSlots = !isFree && (hasAny5h || tier === "PRO" || tier === "ULTRA");

  const toSlot = (key: string, win?: QuotaWindow): AntigravityBucketSlot => {
    const blockedByWeekly = Boolean(win?.blocked_by_weekly);
    const stale = Boolean(win?.stale);
    const rem = blockedByWeekly ? null : (win?.remaining_percent ?? null);
    const used = blockedByWeekly ? null : (win?.used_percent ?? null);
    const hasData = !blockedByWeekly && rem !== null;
    return {
      key,
      label: resolveBucketLabel(key, win?.name || key, lang),
      remainingPercent: rem,
      usedPercent: used,
      resetsAt: win?.resets_at,
      resetInSeconds: win?.reset_in_seconds,
      detail: win?.detail,
      stale,
      blockedByWeekly,
      hasData,
    };
  };

  const buckets: AntigravityBucketSlot[] = [];
  if (hasBuckets) {
    const targetKeys = usePaidSlots ? PAID_BUCKET_KEYS : FREE_BUCKET_KEYS;
    for (const key of targetKeys) {
      buckets.push(toSlot(key, bucketMap.get(key)));
    }
    for (const { key, win } of extraBuckets) {
      buckets.push(toSlot(key, win));
    }
  }

  const isStale =
    extra["quota_summary_stale"] === "true" || buckets.some((b) => b.stale);
  const isForbidden = extra["is_forbidden"] === "true";
  const isNeedsReauth = isAntigravityAccountNeedsReauth(account);
  const errorMessage = extra["quota_error_message"];
  const validationUrl = extra["validation_url"];
  const aiCredits = extra["ai_credits"];

  return {
    buckets,
    models,
    isStale,
    isNeedsReauth,
    isForbidden,
    errorMessage,
    validationUrl,
    aiCredits,
  };
}

export function getAntigravityMiniPills(account: AccountProfile, lang: string): AntigravityMiniPill[] {
  const view = getAntigravityQuotaView(account, lang);
  return view.buckets.map((b) => ({
    key: b.key,
    shortLabel: resolveBucketShortLabel(b.key, b.label),
    percentageText: b.hasData && b.remainingPercent != null ? `${b.remainingPercent}%` : "—",
    quotaClass: getAntigravityQuotaClass(b.remainingPercent, b.hasData),
    blockedByWeekly: b.blockedByWeekly,
    stale: b.stale,
  }));
}
