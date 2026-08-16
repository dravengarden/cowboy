import { assertEquals } from "jsr:@std/assert";
import {
  sessionProviderFacts,
  sessionProviderManageLabel,
  sessionProviderNeedsAttention,
  sessionProviderShowsUsage,
  sessionProviderSummary,
  sessionProviderUsageEmptyMessage,
  sessionProviderUsageRows,
  workspaceOptionsSummary,
} from "./sessionSettingsPresentation.ts";
import { XAI_SIGN_IN_MESSAGE } from "./usageLimits.ts";

const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);

Deno.test("workspace options name the live queue and page-view state", () => {
  assertEquals(
    workspaceOptionsSummary({ queuePaused: false, pageView: false }),
    "Queue running · Conversation",
  );
  assertEquals(
    workspaceOptionsSummary({ queuePaused: true, pageView: true }),
    "Queue paused · Page view",
  );
});

Deno.test("unsigned Providers demand attention until the catalog says they are ready", () => {
  assertEquals(
    sessionProviderNeedsAttention({
      catalogReady: false,
      required: true,
      ready: false,
    }),
    true,
  );
  assertEquals(
    sessionProviderNeedsAttention({
      catalogReady: true,
      required: true,
      ready: false,
    }),
    true,
  );
  assertEquals(
    sessionProviderNeedsAttention({
      catalogReady: true,
      required: true,
      ready: true,
    }),
    false,
  );
  assertEquals(
    sessionProviderNeedsAttention({
      catalogReady: true,
      required: false,
      ready: true,
    }),
    false,
  );
});

Deno.test("signed-in Providers collapse to a brief account summary", () => {
  assertEquals(
    sessionProviderSummary({
      displayName: "Grok",
      required: true,
      ready: true,
      accountLabel: "draven",
      presentation: "account",
    }),
    "Grok · signed in · draven",
  );
  assertEquals(
    sessionProviderSummary({
      displayName: "Codex",
      required: true,
      ready: false,
      presentation: "account",
    }),
    "Codex · signed out",
  );
  assertEquals(
    sessionProviderSummary({
      displayName: "Grok",
      required: false,
      ready: true,
      presentation: "none",
    }),
    "Grok · no sign-in",
  );
});

Deno.test("signed-in session Providers show facts and keep account actions folded", () => {
  assertEquals(
    sessionProviderFacts({
      vendor: "xAI",
      version: "1.1.7",
      accountLabel: "draven",
    }),
    [
      { label: "Vendor", value: "xAI" },
      { label: "Version", value: "1.1.7", mono: true },
      { label: "Account", value: "draven" },
    ],
  );
  assertEquals(sessionProviderManageLabel("account"), "Manage account");
  assertEquals(sessionProviderManageLabel("api_key"), "Manage API key");
  assertEquals(composerSource.includes("sessionProviderFacts"), true);
  assertEquals(composerSource.includes("sessionProviderShowsUsage"), true);
  assertEquals(composerSource.includes("SessionProviderUsage"), true);
  assertEquals(composerSource.includes("setActionsOpen(needsAttention)"), true);
  assertEquals(
    composerSource.includes("<Collapse in={!ready || actionsOpen}"),
    true,
  );
});

Deno.test("signed-in session Providers show this account's usage windows", () => {
  assertEquals(
    sessionProviderShowsUsage({
      ready: true,
      accountUsageProvider: "xai",
    }),
    true,
  );
  assertEquals(
    sessionProviderShowsUsage({
      ready: false,
      accountUsageProvider: "xai",
    }),
    false,
  );
  assertEquals(sessionProviderShowsUsage({ ready: true }), false);

  const weekly = sessionProviderUsageRows({
    provider: "xai",
    status: "available",
    source: "test",
    observed_at_ms: 1,
    rate_limits: {
      config: {
        creditUsagePercent: 37.5,
        currentPeriod: {
          type: "USAGE_PERIOD_TYPE_WEEKLY",
          end: "2026-08-17T00:00:00Z",
        },
      },
    },
  });
  assertEquals(weekly[0]?.label, "Weekly");
  assertEquals(weekly[0]?.value, "63% remaining");
  assertEquals(weekly[0]?.detail?.startsWith("Resets "), true);

  assertEquals(
    sessionProviderUsageRows({
      provider: "deepseek",
      status: "available",
      source: "test",
      observed_at_ms: 1,
      account: {
        accounts: [{
          balanceInfos: [{ currency: "CNY", total_balance: "108.80" }],
        }],
      },
      activity: {
        last24Hours: {
          summary: {
            requests: 2,
            blockingErrors: 0,
            cacheObservations: 2,
            cacheHitTokens: 900,
            cacheMissTokens: 100,
          },
          cost: {
            summary: {
              requests: 1,
              inputTokens: 1_000_000,
              pricedInputTokens: 1_000_000,
              outputTokens: 1_000_000,
              estimatedCny: 2.51,
            },
          },
        },
      },
    }),
    [
      { label: "Balance", value: "¥108.80" },
      { label: "24h spend", value: "¥2.51" },
    ],
  );

  assertEquals(
    sessionProviderUsageEmptyMessage({
      provider: "xai",
      status: "unavailable",
      source: "test",
      observed_at_ms: 1,
      error:
        '_x.ai/billing: {"code":-32000,"message":"Authentication required","data":"Authentication required to fetch billing data"}',
    }),
    XAI_SIGN_IN_MESSAGE,
  );
  assertEquals(
    sessionProviderUsageEmptyMessage(undefined),
    "This account has not exposed usage limits.",
  );
});

Deno.test("session settings collapse queue and page view behind one disclosure", () => {
  assertEquals(composerSource.includes("WorkspaceOptionsSection"), true);
  assertEquals(composerSource.includes("workspaceOptionsSummary"), true);
  assertEquals(
    composerSource.includes('"Expand queue and view"'),
    true,
  );
  assertEquals(composerSource.includes("Queue & view"), true);
  assertEquals(composerSource.includes("SessionProviderSection"), true);
  assertEquals(composerSource.includes("SessionProviderAccess"), true);
});
