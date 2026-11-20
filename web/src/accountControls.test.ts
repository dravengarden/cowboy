import { test } from "bun:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { assert, assertEquals, assertStringIncludes } from "../../tools/test-assert.ts";
import {
  type AccountControl, accountPlanLabel, accountStateBadge, acknowledgementTarget, acknowledgeUncertainEffect, controlActions, controlRequest, logoutStatus,
  pickerUnavailableReason, postAccountControl, sessionParkedReason, sessionPlanKey, signInObservation, validAccountControls, withoutNamedOnly,
} from "./accountControls";
import { AccountControlsPanel } from "./AccountControlsPanel";
import { UncertainEffectAcknowledge } from "./SessionAccountControl";
import { namedOnlyLoginReason, subscriptionAllowed, subscriptionChoices } from "./SessionSubscription";
import { SurfaceProvider } from "./surface/SurfaceProfile";

const render = (element: React.ReactElement) => renderToStaticMarkup(createElement(SurfaceProvider, null, element));

const ACCOUNT = "11111111-1111-4111-8111-111111111111";
const OTHER = "22222222-2222-4222-8222-222222222222";

function control(overrides: Partial<AccountControl> = {}): AccountControl {
  return {
    account_id: ACCOUNT, provider_id: "antigravity", admitted: true, state: "ok", reason: null, until_ms: null,
    revision: 3, fences: [], stop_confirmed: false, uncertain_effects: [], logout: null, ...overrides,
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
  // A Controller before Revision 6 still reports `in_use_by`; it is ignored.
  assert(validAccountControls({ loaded: true, accounts: [{ ...control(), in_use_by: 7 }] }));
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
  // Any number of sessions may run one account (Revision 6): a session
  // another one already runs is still choosable.
  const legacy = { ...control(), in_use_by: "s9" } as AccountControl;
  assertEquals(pickerUnavailableReason(legacy, undefined), undefined);
  // Nothing is preselected: neither an empty choice nor the default account is allowed.
  assert(!subscriptionAllowed(data, "antigravity", ""));
  assert(!subscriptionAllowed(data, "antigravity", "default"));
  assert(subscriptionAllowed(data, "antigravity", ACCOUNT));
  // An unloaded controls snapshot never enables a choice.
  const unloaded = { ...data, account_controls: { ...data.account_controls, loaded: false } };
  assertEquals(subscriptionChoices(unloaded, "antigravity", () => undefined)[0]!.reason, "Account state not loaded");
  // Other Providers keep their default entry.
  const codex = { ...data, registry: { ...data.registry, accounts: [] } };
  assertEquals(subscriptionChoices(codex, "codex", () => undefined).map((choice) => choice.id), ["default"]);
});

test("a parked session says why: uncertain effect or account state, never another session", () => {
  const effect = { session_id: "s1", method: "fs/write_text_file", sequence: 2 };
  const uncertain = sessionParkedReason(control({ uncertain_effects: [effect] }), "s1", undefined);
  assertEquals(uncertain?.uncertain, effect);
  assertStringIncludes(uncertain!.text, "fs/write_text_file");
  const until = Date.UTC(2026, 10, 18, 15, 0);
  assertStringIncludes(sessionParkedReason(control({ state: "rate_limited", until_ms: until }), "s1", "x", until - 1)!.text, "Rate limited until");
  assertEquals(sessionParkedReason(control({ admitted: false }), "s1", "x")!.text, "This account is not admitted for sessions.");
  const legacy = { ...control(), in_use_by: "s2" } as AccountControl;
  assertEquals(sessionParkedReason(legacy, "s1", undefined), undefined);
  assertEquals(sessionParkedReason(legacy, "s1", "x")?.text, "x");
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

test("the account card shows its notice, usage and the kill switch", () => {
  const markup = render(createElement(AccountControlsPanel, {
    control: control({
      state: "blocked", stop_confirmed: true, reason: "owner_block",
      fences: [{ machine_id: "ovh", action: "stop", state: "confirmed" }],
    }),
    observation: {}, disabled: false, onChange: () => undefined,
  }));
  for (const text of [
    "Blocked", "Blocked by the owner", "Usage · Not available",
    "Use of a personal Google account in Cowboy is not supported by Google", "Admitted",
    "Unblock", "Sign out…",
  ]) assertStringIncludes(markup, text);
  assert(!markup.includes("Block…") && !markup.includes("Force release") && !markup.includes("In use by"));
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

test("a named-only account's sign-in is independent of usage collection", () => {
  const observed = (fields: Record<string, unknown>) => [{ account_id: ACCOUNT, provider_id: "antigravity", ...fields }];
  assertEquals(signInObservation({ reason: "account_usage_unsupported", signed_in: true }), "signed_in");
  assertEquals(namedOnlyLoginReason(observed({ reason: "account_usage_unsupported", signed_in: true }), "antigravity", ACCOUNT), undefined);
  assertEquals(namedOnlyLoginReason(observed({ reason: "account_usage_unsupported" }), "antigravity", ACCOUNT), "Sign-in not confirmed");
  assertEquals(namedOnlyLoginReason([], "antigravity", ACCOUNT), "Sign-in not confirmed");
  assertEquals(namedOnlyLoginReason(observed({ reason: "native_login_required" }), "antigravity", ACCOUNT), "Not signed in on a Machine");
});

test("the account card shows the plan the Provider reported, and nothing it cannot read", () => {
  const starter = { id: "free-tier", name: "Antigravity Starter Quota", observed_at_ms: 1 };
  assertEquals(accountPlanLabel(control({ plan: starter })), "Antigravity Starter Quota");
  assertEquals(accountPlanLabel(control()), undefined);
  assertEquals(accountPlanLabel(control({ plan: null })), undefined);
  for (const plan of [
    { id: "Free Tier", name: "Starter" }, { id: "free-tier", name: "" }, { id: "free-tier", name: " Starter" },
    { id: "free-tier", name: "Starter\nUltra" }, { id: "free-tier", name: "Pro\u202eartlU" },
    { id: "free-tier", name: "P".repeat(81) }, { id: 7, name: "Starter" },
    { id: "free-tier", name: "Pro\u061c" }, { id: "free-tier", name: "Pro\u00ad" },
    { id: "free-tier", name: "Pro\ue000" }, { id: "free-tier", name: "Pro\u{10ffff}" },
    { id: "free-tier", name: "Pro\t" }, { id: "free-tier", name: "\u2028Pro" },
    { id: "free-tier", name: "Pro\u3000" }, { id: "free-tier", name: "\u{1F680}".repeat(81) },
  ]) assertEquals(accountPlanLabel(control({ plan: plan as never })), undefined, JSON.stringify(plan));
  // The bound counts code points, like the proxy and the Controller.
  const wide = "\u{1F680}".repeat(80);
  assertEquals(accountPlanLabel(control({ plan: { id: "wide", name: wide } })), wide);
  // A bad plan never invalidates the account's controls.
  assert(validAccountControls({ loaded: true, accounts: [control({ plan: "Ultra" as never })] }));
  const markup = render(createElement(AccountControlsPanel, {
    control: control({ plan: starter }), observation: { available: true }, disabled: false, onChange: () => undefined,
  }));
  for (const text of ["Plan · Antigravity Starter Quota", "data-account-plan=\"free-tier\"", "Usage · Not available"]) {
    assertStringIncludes(markup, text);
  }
  assert(!markup.includes("href"));
  assert(!render(createElement(AccountControlsPanel, {
    control: control(), observation: { available: true }, disabled: false, onChange: () => undefined,
  })).includes("Plan ·"));
});

test("a changed session plan for the account changes its refresh key", () => {
  const reading = (id: string, name: string) => ({ raw: { _meta: { cowboy: { account_plan: { id, name } } } } });
  const sessions = [
    { provider_account_id: ACCOUNT, usage: reading("free-tier", "Starter") },
    { provider_account_id: ACCOUNT, usage: reading("free-tier", "Starter") },
    { provider_account_id: "other", usage: reading("ultra", "Ultra") },
    { provider_account_id: ACCOUNT, usage: { raw: null } },
    { provider_account_id: ACCOUNT },
  ];
  const key = sessionPlanKey(sessions, ACCOUNT);
  assertEquals(sessionPlanKey([], ACCOUNT), "");
  // Another account's plan, or a repeated one, changes nothing.
  assertEquals(sessionPlanKey([...sessions, { provider_account_id: "other", usage: reading("pro", "Pro") }], ACCOUNT), key);
  assertEquals(sessionPlanKey([...sessions].reverse(), ACCOUNT), key);
  assert(sessionPlanKey([...sessions, { provider_account_id: ACCOUNT, usage: reading("g1-pro-tier", "Pro") }], ACCOUNT) !== key);
  assert(sessionPlanKey([{ provider_account_id: ACCOUNT, usage: reading("free-tier", "Starter 2") }], ACCOUNT) !== key);
});

test("a named-only Provider never offers a Service account card or sign-in", () => {
  const entries = [{ provider_id: "antigravity" }, { provider_id: "codex" }, { provider_id: "claude-code" }];
  assertEquals(withoutNamedOnly(entries, ["antigravity"]).map((entry) => entry.provider_id), ["codex", "claude-code"]);
  // Before the snapshot loads, or from a Controller that predates the field.
  assertEquals(withoutNamedOnly(entries, undefined).length, 3);
  assertEquals(withoutNamedOnly(entries, [""]).length, 3);
});
