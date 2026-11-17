import { expect, test } from "bun:test";
import { serviceSignInBrowser } from "./serviceSignInBrowser";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";

function snapshot(overrides: Partial<SubscriptionSnapshot> = {}): SubscriptionSnapshot {
  return {
    dataset: { dataset_id: "ds" },
    machine_browser: "enabled",
    legacy_browsers: [
      { card: "anthropic", account_id: "key-anthropic", providers: ["claude-code"], browser_enabled: true, available: true, share_id: "heron" },
      { card: "deepseek", account_id: "key-deepseek", providers: ["claude-deepseek", "codex-deepseek"], browser_enabled: false, available: true },
      { card: "xai", account_id: "key-xai", providers: ["grok"], browser_enabled: true, available: false },
    ],
    ...overrides,
  } as unknown as SubscriptionSnapshot;
}

test("an unknown inventory or owner opens the page nowhere", () => {
  expect(serviceSignInBrowser(undefined, "me", "claude-code")).toEqual({ kind: "unknown" });
  expect(serviceSignInBrowser(snapshot(), undefined, "claude-code")).toEqual({ kind: "unknown" });
});

test("a card with a Machine browser keeps its sign-in page in that browser", () => {
  expect(serviceSignInBrowser(snapshot(), "me", "claude-code")).toEqual({
    kind: "machine", owner: "me", dataset: "ds", account: "key-anthropic",
  });
});

test("a disabled card browser still refuses the device page", () => {
  // The entry explains how to enable the browser instead of falling back.
  expect(serviceSignInBrowser(snapshot(), "me", "codex-deepseek")).toEqual({
    kind: "machine", owner: "me", dataset: "ds", account: "key-deepseek",
  });
});

test("only a Service or card without a Machine browser keeps the device page", () => {
  expect(serviceSignInBrowser(snapshot({ machine_browser: "disabled" } as Partial<SubscriptionSnapshot>), "me", "claude-code"))
    .toEqual({ kind: "device" });
  expect(serviceSignInBrowser(snapshot(), "me", "grok")).toEqual({ kind: "device" });
  expect(serviceSignInBrowser(snapshot(), "me", "codex")).toEqual({ kind: "device" });
});
