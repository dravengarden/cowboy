import { test } from "bun:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { assert, assertEquals, assertStringIncludes } from "../../tools/test-assert.ts";
import {
  type AccountControl, accountStateBadge, acknowledgementTarget, acknowledgeUncertainEffect, controlActions, controlRequest, logoutStatus,
  pickerUnavailableReason, postAccountControl, sessionParkedReason, signInObservation, validAccountControls,
} from "./accountControls";
import { AccountControlsPanel } from "./AccountControlsPanel";
import { UncertainEffectAcknowledge } from "./SessionAccountControl";
import { subscriptionAllowed, subscriptionChoices } from "./SessionSubscription";
import { SurfaceProvider } from "./surface/SurfaceProfile";

const render = (element: React.ReactElement) => renderToStaticMarkup(createElement(SurfaceProvider, null, element));

const ACCOUNT = "11111111-1111-4111-8111-111111111111";
const OTHER = "22222222-2222-4222-8222-222222222222";

function control(overrides: Partial<AccountControl> = {}): AccountControl {
  return {
    account_id: ACCOUNT, provider_id: "antigravity", admitted: true, state: "ok", reason: null, until_ms: null,
    revision: 3, in_use_by: null, fences: [], stop_confirmed: false, uncertain_effects: [], logout: null, ...overrides,
  };
}

test("the account card names every state, its reason and the rate-limit end", () => {
  assertEquals(accountStateBadge(control(), "signed_in").label, "Signed in");
  assertEquals(accountStateBadge(control(), "not_signed_in").label, "Not signed in");
  for (const [state, label] of [
    ["verification_required", "Verification required"], ["location_unsupported", "Location unsupported"],
    ["suspended", "Suspended"], ["blocked", "Blocked"], ["needs_attention", "Needs attention"],
    ["unknown_refusal", "Unknown refusal"], ["something_new", "Unknown refusal"],
  ]) assertEquals(accountStateBadge(control({ state: state! }), "signed_in").label, label, state);
  const until = Date.UTC(2026, 10, 18, 15, 0);
  const limited = accountStateBadge(control({ state: "rate_limited", until_ms: until }), "signed_in", until - 1000);
  assertStringIncludes(limited.label, "Rate limited until");
  assertStringIncludes(accountStateBadge(control({ state: "rate_limited", until_ms: until }), undefined, until + 1).label, "send again");
  assertEquals(accountStateBadge(control({ state: "needs_attention", reason: "policy_violation:read_file" }), undefined).detail,
    "A built-in tool ran (read_file)");
  assertEquals(signInObservation({ reason: "native_login_required" }), "not_signed_in");
  assertEquals(signInObservation({ available: true }), "signed_in");
  assertEquals(signInObservation(undefined), undefined);
});

test("only an affirmative sign-in observation shows the account signed in", () => {
  // Loading, or a read without a resolved native login, is unknown.
  assertEquals(signInObservation({}), undefined);
  assertEquals(signInObservation({ reason: "account_disabled" }), undefined);
  assertEquals(signInObservation({ reason: "something_new", available: false }), undefined);
  const unknown = accountStateBadge(control(), signInObservation(undefined));
  assertEquals([unknown.label, unknown.tone], ["Sign-in unknown", "default"]);
  assertEquals(accountStateBadge(control(), signInObservation({ available: true })).label, "Signed in");
});

test("an acknowledgement sheet acknowledges only the effect it showed", () => {
  const shown = { session: "s1", method: "fs/write_text_file", sequence: 2 };
  const effect = (sequence: number, method = "fs/write_text_file") => ({ session_id: "s1", method, sequence });
  assertEquals(acknowledgementTarget(shown, "s1", effect(2)), shown);
  // Nothing shown yet, the effect resolved, a newer sequence, another
  // method, or the sheet now rendered for another session: stale.
  assertEquals(acknowledgementTarget(undefined, "s1", effect(2)), undefined);
  assertEquals(acknowledgementTarget(shown, "s1", undefined), undefined);
  assertEquals(acknowledgementTarget(shown, "s1", effect(3)), undefined);
  assertEquals(acknowledgementTarget(shown, "s1", effect(2, "terminal/create")), undefined);
  assertEquals(acknowledgementTarget(shown, "s2", effect(2)), undefined);
});

test("kill switch actions follow the block, the confirmed stop and the logout", () => {
  assertEquals(controlActions(control()), { clear: false, block: true, unblock: false, logout: false, stopUnits: false });
  assertEquals(controlActions(control({ state: "verification_required" })).clear, true);
  const stopping = control({ state: "blocked", fences: [{ machine_id: "ovh", action: "stop", state: "pending" }] });
  assertEquals(controlActions(stopping), { clear: false, block: false, unblock: true, logout: false, stopUnits: true });
  const stopped = control({
    state: "blocked", stop_confirmed: true, fences: [{ machine_id: "ovh", action: "stop", state: "confirmed" }],
    in_use_by: "s1",
  });
  assertEquals(controlActions(stopped), { clear: false, block: false, unblock: true, logout: true, stopUnits: false });
  const signingOut = { ...stopped, logout: { operation_id: "op", revision: 3, requested_at_ms: 1, state: "pending",
    targets: [{ machine_id: "ovh", state: "pending" }] } };
  assertEquals(controlActions(signingOut).logout, false);
  assertEquals(logoutStatus(signingOut.logout), "Signing out · waiting for ovh");
  assertEquals(logoutStatus({ ...signingOut.logout, state: "signed_out" }), "Signed out · history kept");
  // A logout of an earlier block does not hide the action of this one.
  assertEquals(controlActions({ ...signingOut, revision: 5 }).logout, true);
  assertEquals(controlRequest("logout", stopped), { action: "logout", confirm: true });
  assertEquals(controlRequest("unblock", stopped), { action: "unblock", expected_revision: 3 });
  assertEquals(controlRequest("stop_units", stopped), { action: "stop_units" });
});

test("the read model is validated before the card trusts it", () => {
  assert(validAccountControls({ loaded: true, accounts: [control()] }));
  assert(!validAccountControls({ loaded: true, accounts: [{ ...control(), fences: [{}] }] }));
  assert(!validAccountControls({ accounts: [] }));
  assert(!validAccountControls({ loaded: true, accounts: [{ ...control(), in_use_by: 7 }] }));
});

test("the new-session picker offers only admitted, signed-in, ok accounts and never a default", () => {
  const data = {
    registry: { accounts: [
      { id: ACCOUNT, provider_id: "antigravity", label: "Work", enabled: true },
      { id: OTHER, provider_id: "antigravity", label: "Home", enabled: true },
      { id: "33333333-3333-4333-8333-333333333333", provider_id: "antigravity", label: "Spare", enabled: true },
    ], defaults: {} },
    session_admission: "enabled", session_providers: ["antigravity"], named_only_providers: ["antigravity"],
    account_controls: { loaded: true, accounts: [
      control(), control({ account_id: OTHER, state: "blocked" }),
      control({ account_id: "33333333-3333-4333-8333-333333333333", admitted: false }),
    ] },
  };
  const choices = subscriptionChoices(data, "antigravity", (id) => id === ACCOUNT ? undefined : undefined);
  assertEquals(choices.map((choice) => choice.id), [ACCOUNT, OTHER, "33333333-3333-4333-8333-333333333333"]);
  assertEquals(choices.map((choice) => choice.reason), [undefined, "Blocked", "Not admitted for sessions"]);
  assert(!choices.some((choice) => choice.label.includes("Default")));
  const signedOut = subscriptionChoices(data, "antigravity", () => "Not signed in on a Machine");
  assertEquals(signedOut[0]!.reason, "Not signed in on a Machine");
  assertEquals(pickerUnavailableReason(undefined, undefined), "Account state not loaded");
  assertEquals(pickerUnavailableReason(control({ in_use_by: "s9" }), undefined),
    "In use by another session (s9); hibernate it first");
  // Nothing is preselected: neither an empty choice nor the default account is allowed.
  assert(!subscriptionAllowed(data, "antigravity", ""));
  assert(!subscriptionAllowed(data, "antigravity", "default"));
  assert(subscriptionAllowed(data, "antigravity", ACCOUNT));
  // Other Providers keep their default entry.
  const codex = { ...data, registry: { ...data.registry, accounts: [] } };
  assertEquals(subscriptionChoices(codex, "codex", () => undefined).map((choice) => choice.id), ["default"]);
});

test("a parked session says why: uncertain effect, account state, another session's runtime", () => {
  const effect = { session_id: "s1", method: "fs/write_text_file", sequence: 2 };
  const uncertain = sessionParkedReason(control({ uncertain_effects: [effect] }), "s1", undefined);
  assertEquals(uncertain?.uncertain, effect);
  assertStringIncludes(uncertain!.text, "fs/write_text_file");
  const until = Date.UTC(2026, 10, 18, 15, 0);
  assertStringIncludes(sessionParkedReason(control({ state: "rate_limited", until_ms: until }), "s1", "x", until - 1)!.text, "Rate limited until");
  assertEquals(sessionParkedReason(control({ admitted: false }), "s1", "x")!.text, "This account is not admitted for sessions.");
  const busy = control({ in_use_by: "s2" });
  assertEquals(sessionParkedReason(busy, "s1", "x")?.waitingFor, "s2");
  assertEquals(sessionParkedReason(busy, "s2", undefined), undefined);
  assertEquals(sessionParkedReason(control(), "s1", "Waiting for resources: account controls are loading")?.text,
    "account controls are loading");
});

test("acknowledging posts exactly the current sequence; a refusal is explained", async () => {
  const original = globalThis.fetch;
  const requests: { url: string; body: unknown }[] = [];
  let status = 200;
  let code = "";
  globalThis.fetch = (async (url: string, init?: RequestInit) => {
    requests.push({ url, body: JSON.parse(String(init?.body)) });
    return new Response(JSON.stringify(status === 200 ? { status: "committed" } : { error: { code } }), { status });
  }) as typeof fetch;
  try {
    await acknowledgeUncertainEffect("s1", 2);
    assertEquals(requests.at(-1), { url: "/api/sessions/s1/uncertain-effect/acknowledge", body: { sequence: 2 } });
    status = 409; code = "uncertain_effect_changed";
    let message = "";
    try { await acknowledgeUncertainEffect("s1", 2); } catch (cause) { message = (cause as Error).message; }
    assertStringIncludes(message, "Another uncertain effect arrived");
    code = "account_stop_unconfirmed";
    try { await postAccountControl(ACCOUNT, "logout", control({ state: "blocked" })); } catch (cause) { message = (cause as Error).message; }
    assertEquals(requests.at(-1), { url: `/api/provider-accounts/${ACCOUNT}/controls`, body: { action: "logout", confirm: true } });
    assertStringIncludes(message, "confirmed stopped");
  } finally { globalThis.fetch = original; }
});

test("the account card shows its notice, usage, the session using it and the kill switch", () => {
  const markup = render(createElement(AccountControlsPanel, {
    control: control({
      state: "blocked", stop_confirmed: true, reason: "owner_block",
      fences: [{ machine_id: "ovh", action: "stop", state: "confirmed" }],
      in_use_by: "s1",
    }),
    observation: {}, disabled: false, onChange: () => undefined,
  }));
  for (const text of [
    "Blocked", "Blocked by the owner", "In use by", "s1", "Usage · Not available",
    "Use of a personal Google account in Cowboy is not supported by Google", "Admitted",
    "Unblock", "Sign out…",
  ]) assertStringIncludes(markup, text);
  assert(!markup.includes("Block…") && !markup.includes("Force release"));
  const ok = render(createElement(AccountControlsPanel, {
    control: control({ admitted: false }), observation: { reason: "native_login_required" }, disabled: false, onChange: () => undefined,
  }));
  for (const text of ["Not signed in", "Not admitted", "Block…"]) assertStringIncludes(ok, text);
  assert(!ok.includes("Sign out…") && !ok.includes("In use by") && !ok.includes("Unblock"));
  const ack = render(createElement(UncertainEffectAcknowledge, {
    session: "s1", effect: { session_id: "s1", method: "fs/write_text_file", sequence: 2 }, onDone: () => undefined,
  }));
  assertStringIncludes(ack, "Acknowledge…");
});
