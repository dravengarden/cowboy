import { expect, test } from "bun:test";
import { serviceSignInBrowser } from "./serviceSignInBrowser";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";

function snapshot(overrides: Partial<SubscriptionSnapshot> = {}): SubscriptionSnapshot {
  return { dataset: { dataset_id: "ds" }, machine_browser: "enabled", ...overrides } as unknown as SubscriptionSnapshot;
}

test("an unknown inventory or owner opens the page nowhere", () => {
  expect(serviceSignInBrowser(undefined, "me", "claude-code")).toEqual({ kind: "unknown" });
  expect(serviceSignInBrowser(snapshot(), undefined, "claude-code")).toEqual({ kind: "unknown" });
});

test("the retired legacy card browsers never take a Service sign-in page", () => {
  for (const provider of ["claude-code", "codex-deepseek", "grok", "codex"]) {
    expect(serviceSignInBrowser(snapshot(), "me", provider)).toEqual({ kind: "device" });
  }
  expect(serviceSignInBrowser(snapshot({ machine_browser: "not_enabled" }), "me", "claude-code"))
    .toEqual({ kind: "device" });
});
