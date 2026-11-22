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

test("a retired card's Service sign-in never opens on this device", () => {
  // A current Controller names the retired Providers.
  const current = snapshot({ retired_default_providers: ["claude-code", "codex", "grok", "codex-deepseek"] });
  for (const provider of ["claude-code", "codex-deepseek", "grok", "codex"]) {
    expect(serviceSignInBrowser(current, "me", provider)).toEqual({ kind: "refused" });
  }
  // An older Controller does not, but its family table does.
  const families = { version: 3, providers: { "claude-code": "anthropic", codex: "openai", antigravity: "google" } };
  const older = snapshot({ machine_browser: "not_enabled", browser_share_families: families });
  expect(serviceSignInBrowser(older, "me", "claude-code")).toEqual({ kind: "refused" });
  // A Provider that never had a card keeps the device page.
  expect(serviceSignInBrowser(older, "me", "antigravity")).toEqual({ kind: "device" });
  expect(serviceSignInBrowser(current, "me", "future-provider")).toEqual({ kind: "device" });
});

test("an older Controller's card Machine browser takes the sign-in page", () => {
  const legacy = { legacy_browsers: [{ account_id: "lynx-key", providers: ["claude-code"], available: true },
    { account_id: "pika-key", providers: ["codex"], available: false }, { garbled: true }] };
  const older = snapshot(legacy as Partial<SubscriptionSnapshot>);
  expect(serviceSignInBrowser(older, "me", "claude-code"))
    .toEqual({ kind: "machine", owner: "me", dataset: "ds", account: "lynx-key" });
  // An unavailable browser, or a Service without Machine browsers, never
  // falls back to this device.
  const families = { version: 3, providers: { "claude-code": "anthropic", codex: "openai" } };
  expect(serviceSignInBrowser(snapshot({ ...legacy, browser_share_families: families } as Partial<SubscriptionSnapshot>), "me", "codex"))
    .toEqual({ kind: "refused" });
  expect(serviceSignInBrowser(snapshot({ ...legacy, machine_browser: "not_enabled", browser_share_families: families } as Partial<SubscriptionSnapshot>), "me", "claude-code"))
    .toEqual({ kind: "refused" });
});
