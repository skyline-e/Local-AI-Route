import assert from "node:assert/strict";
import test from "node:test";
import type { AccountProfile } from "../src/types.ts";
import { getAntigravityMiniPills, getAntigravityQuotaView } from "../src/antigravityQuota.ts";

function makeAccount(
  windows: NonNullable<AccountProfile["quota"]>["windows"],
  opts?: {
    plan_type?: string;
    extra_info?: Record<string, string>;
  },
): AccountProfile {
  return {
    id: "acc-1",
    provider: "antigravity",
    name: "dev",
    email: "dev@example.com",
    plan_type: opts?.plan_type,
    created_at: "2026-11-22 10:00:00",
    is_active: true,
    quota: {
      provider: "antigravity",
      account_id: "acc-1",
      email: "dev@example.com",
      plan_type: opts?.plan_type,
      fetched_at: "2026-11-22 10:00:00",
      windows,
      extra_info: opts?.extra_info || {},
    },
  };
}

test("paid account renders 4 standard slots with missing windows as no-data rather than 100%", () => {
  const acct = makeAccount(
    [
      {
        name: "Gemini · 5h",
        remaining_percent: 25,
        used_percent: 75,
        window_key: "gemini:5h",
        window_kind: "bucket",
      },
    ],
    {
      plan_type: "Pro",
      extra_info: { subscription_tier: "PRO", ai_credits: "25" },
    },
  );

  const view = getAntigravityQuotaView(acct, "zh-CN");
  assert.equal(view.aiCredits, "25");
  assert.equal(view.buckets.length, 4);
  assert.equal(view.buckets[0].key, "gemini:5h");
  assert.equal(view.buckets[0].hasData, true);
  assert.equal(view.buckets[0].remainingPercent, 25);

  for (const missing of view.buckets.slice(1)) {
    assert.equal(missing.hasData, false);
    assert.equal(missing.remainingPercent, null);
  }
});

test("free account omits 5h slots and keeps weekly + extra pools", () => {
  const acct = makeAccount(
    [
      {
        name: "Gemini · Weekly",
        remaining_percent: 80,
        used_percent: 20,
        window_key: "gemini:weekly",
        window_kind: "bucket",
      },
      {
        name: "Gemini 4 Argon · Weekly",
        remaining_percent: 95,
        used_percent: 5,
        window_key: "argon:weekly",
        window_kind: "bucket",
      },
      {
        name: "Gemini 4 Hydrogen (High)",
        remaining_percent: 100,
        used_percent: 0,
        window_key: "gemini-4-hydrogen-high",
        window_kind: "model",
      },
    ],
    {
      plan_type: "Free",
      extra_info: { subscription_tier: "FREE" },
    },
  );

  const view = getAntigravityQuotaView(acct, "en-US");
  assert.deepEqual(
    view.buckets.map((b) => [b.key, b.hasData, b.remainingPercent]),
    [
      ["gemini:weekly", true, 80],
      ["claude:weekly", false, null],
      ["argon:weekly", true, 95],
    ],
  );
  assert.equal(view.buckets.some((b) => b.key.endsWith(":5h")), false);
  assert.deepEqual(
    view.models.map((m) => [m.key, m.remainingPercent]),
    [["gemini-4-hydrogen-high", 100]],
  );
});

test("blocked claude:5h displays weeklyLimitReached state without stale percentage", () => {
  const acct = makeAccount(
    [
      {
        name: "Claude · 5h",
        remaining_percent: null,
        used_percent: null,
        window_key: "claude:5h",
        window_kind: "bucket",
        blocked_by_weekly: true,
      },
      {
        name: "Claude · Weekly",
        remaining_percent: 0,
        used_percent: 100,
        window_key: "claude:weekly",
        window_kind: "bucket",
        summary_exhausted: true,
      },
    ],
    {
      plan_type: "Pro",
      extra_info: { subscription_tier: "PRO" },
    },
  );

  const pills = getAntigravityMiniPills(acct, "zh-CN");
  const claude5h = pills.find((p) => p.key === "claude:5h")!;
  assert.equal(claude5h.blockedByWeekly, true);
  assert.equal(claude5h.percentageText, "—");
});

test("reauth and stale flags surface validation url cleanly", () => {
  const acct = makeAccount(
    [
      {
        name: "Gemini · Weekly",
        remaining_percent: 45,
        used_percent: 55,
        window_key: "gemini:weekly",
        window_kind: "bucket",
        stale: true,
      },
    ],
    {
      plan_type: "Free",
      extra_info: {
        subscription_tier: "FREE",
        quota_summary_stale: "true",
        quota_error_reason: "VALIDATION_REQUIRED",
        quota_error_message: "Verify your account",
        validation_url: "https://accounts.google.com/signin/continue?authuser=dev%40example.com",
      },
    },
  );

  const view = getAntigravityQuotaView(acct, "zh-CN");
  assert.equal(view.isStale, true);
  assert.equal(view.isNeedsReauth, true);
  assert.match(view.validationUrl || "", /accounts\.google\.com/);
});
