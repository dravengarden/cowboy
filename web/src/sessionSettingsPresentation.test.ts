import { assertEquals } from "jsr:@std/assert";
import {
  sessionProviderNeedsAttention,
  sessionProviderSummary,
  workspaceOptionsSummary,
} from "./sessionSettingsPresentation.ts";

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
