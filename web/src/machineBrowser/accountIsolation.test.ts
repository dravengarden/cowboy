import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import type { SubscriptionSnapshot } from "../subscriptionAccounts";
import {
  type BrowserRead,
  type BrowserStateEvent,
  type BrowserTarget,
  browserStreamUrl,
  createMachineBrowserApi,
  validBrowserRead,
} from "./api";
import {
  type AccountBrowserModel,
  INITIAL_BROWSER_MODEL,
  linkEntry,
  linkTarget,
  modelFor,
  pinnedMachine,
  readTicket,
  reduceAccountBrowser,
  reduceBrowser,
  resetTarget,
  shownTarget,
} from "./browserModel";
import { validSubscriptionSnapshot } from "../subscriptionAccounts";
import { type ViewerControls, addressCommand, chromeState, controlsFor } from "./browserChrome";
import {
  accountBrowserEntry,
  legacyBrowserOfProvider,
  linkBrowserOffered,
  machineChoices,
  needsMachineChoice,
} from "./entryRules";

const A = "11111111-2222-4333-8444-555555555555";
const B = "66666666-7777-4888-9999-000000000000";

function running(account: string, instance: string, incarnation: number): BrowserStateEvent {
  return {
    instance: { account_id: account, instance_id: instance, incarnation },
    state: "detached",
    profile_revision: 3,
    observed_at_ms: 1,
  };
}

const openA = { machine_id: "hawk", state: running(A, "b-aaaaaaaaaaaaaaaaaaaaaaaa", 7) };
const openB = { machine_id: "ovh", state: running(B, "b-bbbbbbbbbbbbbbbbbbbbbbbb", 2) };

test("opening account A's panel then account B's never carries A's browser into B", () => {
  let state: AccountBrowserModel = { account: A, model: INITIAL_BROWSER_MODEL };
  const send = (account: string, event: Parameters<typeof reduceAccountBrowser>[1]) => {
    state = reduceAccountBrowser(state, event);
    return modelFor(state, account);
  };
  // A opens on hawk, the viewer mounts and the remote browser copies text.
  send(A, { kind: "event", account: A, event: { type: "busy", action: "open" } });
  send(A, {
    kind: "event",
    account: A,
    event: { type: "result", action: "open", result: openA, now: 1 },
  });
  let a = send(A, {
    kind: "event",
    account: A,
    event: { type: "remote_clipboard", text: "secret-of-A" },
  });
  assertEquals(shownTarget(a)?.machine_id, "hawk");
  assertEquals(a.remoteClipboard, "secret-of-A");

  // Before the panel is bound to B, B is shown nothing of A's.
  assertEquals(modelFor(state, B), INITIAL_BROWSER_MODEL);
  let b = send(B, { kind: "bind", account: B });
  assertEquals(b, INITIAL_BROWSER_MODEL);
  assertEquals(b.remoteClipboard, undefined);
  assertEquals(shownTarget(b), undefined);
  // B's first read is not pinned to A's Machine, and nothing can be reset
  // or commanded with A's revision or instance.
  assertEquals(pinnedMachine(b), undefined);
  assertEquals(readTicket({ current: 0 }, () => b).pinned, undefined);
  assertEquals(resetTarget(b.read), undefined);

  // Late answers and viewer callbacks of A are discarded, not applied to B.
  for (const event of [
    { type: "result", action: "open", result: openA, now: 2 },
    { type: "read", read: { ...openA, placements: [], admission: null }, now: 2 },
    { type: "stream_state", state: openA.state },
    { type: "remote_clipboard", text: "late-secret-of-A" },
    { type: "attach", takeover: true },
  ] as const) {
    b = send(B, { kind: "event", account: A, event });
  }
  assertEquals(b, INITIAL_BROWSER_MODEL);

  // B's own open attaches only to B's browser, and the stream names B's
  // account, Machine, instance and incarnation.
  b = send(B, {
    kind: "event",
    account: B,
    event: { type: "result", action: "open", result: openB, now: 3 },
  });
  const target = shownTarget(b);
  assertEquals(target?.machine_id, "ovh");
  assertEquals(target?.instance.instance_id, "b-bbbbbbbbbbbbbbbbbbbbbbbb");
  assertEquals(b.remoteClipboard, undefined);
  const url = new URL(
    browserStreamUrl(B, target!, { width: 800, height: 600, scale: 1 }, false, {
      protocol: "https:",
      host: "cowboy.test",
    }),
  );
  assertEquals(url.pathname, `/api/provider-accounts/${B}/browser/stream`);
  assertEquals(url.searchParams.get("machine_id"), "ovh");
  assertEquals(url.searchParams.get("instance_id"), "b-bbbbbbbbbbbbbbbbbbbbbbbb");
  assertEquals(url.searchParams.get("incarnation"), "2");

  // Returning to A starts A from nothing too: B's browser is not A's.
  a = send(A, { kind: "bind", account: A });
  assertEquals(a, INITIAL_BROWSER_MODEL);
  // Re-binding the bound account keeps its state.
  const before = state;
  send(A, { kind: "bind", account: A });
  assertEquals(state === before, true);
});

function snapshot(
  account: Partial<SubscriptionSnapshot["registry"]["accounts"][number]>,
  gate: "enabled" | "not_enabled" = "enabled",
): SubscriptionSnapshot {
  return {
    dataset: { dataset_id: "d", user_id: "u" },
    registry: {
      revision: 1,
      defaults: {},
      accounts: [{ id: A, provider_id: "claude-code", label: "Test", enabled: true, ...account }],
    },
    session_admission: "enabled",
    machine_browser: gate,
  };
}

test("usage-card entries follow the gate and the account's own enablement", () => {
  assertEquals(accountBrowserEntry(snapshot({ browser_enabled: true }), A).kind, "ready");
  const off = accountBrowserEntry(snapshot({ browser_enabled: true }, "not_enabled"), A);
  assertEquals(off.kind === "disabled" && !off.manage, true);
  const notEnabled = accountBrowserEntry(snapshot({}), A);
  assertEquals(notEnabled.kind === "disabled" && notEnabled.manage, true);
  assertEquals(
    accountBrowserEntry(snapshot({ browser_enabled: true, enabled: false }), A).kind,
    "disabled",
  );
  // Service-collected Provider cards (the legacy default account) and
  // accounts being revoked offer no browser.
  for (const id of ["anthropic", "openai", "default"]) {
    assertEquals(accountBrowserEntry(snapshot({ browser_enabled: true }), id).kind, "none");
  }
  assertEquals(
    accountBrowserEntry(snapshot({ browser_enabled: true, revoking: true }), A).kind,
    "none",
  );
  assertEquals(accountBrowserEntry(undefined, A).kind, "none");
});

// Legacy default usage cards, each with its own browser key (UUID version 8).
const LYNX = "aaaaaaaa-bbbb-8ccc-8ddd-eeeeeeeeeeee";
const MOLE = "12345678-9abc-8def-9012-3456789abcde";
const PIKA = "0f0f0f0f-1e1e-8d2d-ac3c-4b4b4b4b4b4b";

function legacySnapshot(
  enabled: Partial<Record<string, boolean>> = {},
  gate: "enabled" | "not_enabled" = "enabled",
): SubscriptionSnapshot {
  const card = (name: string, key: string, providers: string[], available = true) => ({
    card: name,
    account_id: key,
    providers,
    browser_enabled: enabled[name] === true,
    available,
  });
  return {
    ...snapshot({ browser_enabled: true }, gate),
    legacy_browsers: [
      card("anthropic", LYNX, ["claude-code"]),
      card("deepseek", MOLE, ["claude-deepseek", "codex-deepseek", "deepseek-harness"]),
      card("openai", PIKA, ["codex"]),
      card("xai", "99999999-8888-8777-a666-555555555555", ["grok"], false),
    ],
  };
}

test("each legacy default card opens its own browser key, never another card's or a named account's", () => {
  const accounts = legacySnapshot({ anthropic: true, deepseek: true, openai: true });
  assertEquals(validSubscriptionSnapshot(accounts, "u"), true);
  const keys = new Set<string>();
  for (const [card, key] of [
    ["anthropic", LYNX],
    ["deepseek", MOLE],
    ["openai", PIKA],
  ] as const) {
    const entry = accountBrowserEntry(accounts, card, `Card ${card}`);
    assertEquals(entry.kind, "ready");
    if (entry.kind !== "ready") continue;
    assertEquals(entry.account, {
      id: key,
      label: `Card ${card}`,
      browser_enabled: true,
      legacy: true,
    });
    keys.add(entry.account.id);
  }
  const named = accountBrowserEntry(accounts, A);
  assertEquals(
    named.kind === "ready" && named.account.legacy === false && named.account.id === A,
    true,
  );
  keys.add(A);
  assertEquals(keys.size, 4);
  // The DeepSeek card is one card collected for three Providers: one
  // browser, controlled from its first Provider's row only.
  assertEquals(legacyBrowserOfProvider(accounts, "claude-deepseek")?.account_id, MOLE);
  assertEquals(legacyBrowserOfProvider(accounts, "codex-deepseek"), undefined);
  assertEquals(legacyBrowserOfProvider(accounts, "deepseek-harness"), undefined);
  assertEquals(legacyBrowserOfProvider(accounts, "claude-code")?.account_id, LYNX);
  // A card the Service no longer collects, and the shared default identity,
  // offer nothing.
  assertEquals(accountBrowserEntry(accounts, "xai").kind, "none");
  assertEquals(accountBrowserEntry(accounts, "default").kind, "none");
  assertEquals(
    validSubscriptionSnapshot({ ...accounts, legacy_browsers: [{ card: "anthropic" }] }, "u"),
    false,
  );
});

test("a legacy card's browser stays disabled until its owner enables that card", () => {
  // Lynx is off by default: disabled with the reason and a link to account
  // management, while another card's enablement changes nothing for it.
  const lynx = accountBrowserEntry(
    legacySnapshot({ deepseek: true }),
    "anthropic",
    "Claude · Lynx",
  );
  assertEquals(lynx.kind === "disabled" && lynx.manage, true);
  assertEquals(lynx.kind === "disabled" && lynx.account.id === LYNX, true);
  assertEquals(accountBrowserEntry(legacySnapshot({ deepseek: true }), "deepseek").kind, "ready");
  assertEquals(accountBrowserEntry(legacySnapshot({ anthropic: true }), "anthropic").kind, "ready");
  // The Service gate still applies to legacy cards.
  const gated = accountBrowserEntry(
    legacySnapshot({ anthropic: true }, "not_enabled"),
    "anthropic",
  );
  assertEquals(gated.kind === "disabled" && !gated.manage, true);
});

test("a legacy card's panel never shows another card's browser", () => {
  let state: AccountBrowserModel = { account: LYNX, model: INITIAL_BROWSER_MODEL };
  const openLynx = { machine_id: "ovh", state: running(LYNX, "b-llllllllllllllllllllllll", 1) };
  state = reduceAccountBrowser(state, {
    kind: "event",
    account: LYNX,
    event: { type: "result", action: "open", result: openLynx, now: 1 },
  });
  assertEquals(shownTarget(modelFor(state, LYNX))?.machine_id, "ovh");
  state = reduceAccountBrowser(state, { kind: "bind", account: MOLE });
  assertEquals(modelFor(state, MOLE), INITIAL_BROWSER_MODEL);
  state = reduceAccountBrowser(state, {
    kind: "event",
    account: LYNX,
    event: { type: "result", action: "open", result: openLynx, now: 2 },
  });
  assertEquals(modelFor(state, MOLE), INITIAL_BROWSER_MODEL);
  // A card signed out everywhere has no credential Machine: the owner chooses.
  const read: BrowserRead = {
    machine_id: null,
    state: null,
    placements: [],
    admission: null,
    choose_machine: true,
    capable_machines: [{ machine_id: "ovh" }],
  };
  assertEquals(needsMachineChoice(read), true);
});

test("a never-signed-in account chooses among the offered Machines", () => {
  const read: BrowserRead = {
    machine_id: null,
    state: null,
    placements: [],
    admission: null,
    choose_machine: true,
    capable_machines: [{ machine_id: "hawk" }, { machine_id: "ovh" }],
  };
  assertEquals(validBrowserRead(read), true);
  assertEquals(validBrowserRead({ ...read, capable_machines: [{}] }), false);
  assertEquals(validBrowserRead({ ...read, choose_machine: "yes" }), false);
  assertEquals(needsMachineChoice(read), true);
  assertEquals(needsMachineChoice({ ...read, choose_machine: false }), false);
  assertEquals(needsMachineChoice({ ...read, admission: "browser_not_enabled" }), false);
  assertEquals(needsMachineChoice(undefined), false);
  assertEquals(machineChoices(read, [{ id: "hawk", display_name: "Hawk" }]), [
    { id: "hawk", label: "Hawk" },
    { id: "ovh", label: "ovh" },
  ]);
});

test("a sign-in link is offered only for the account's admitted browser on the login's Machine", () => {
  const read: BrowserRead = {
    machine_id: null,
    state: null,
    placements: [],
    admission: null,
    choose_machine: true,
    capable_machines: [{ machine_id: "hawk" }],
  };
  assertEquals(linkBrowserOffered(read, "hawk"), true);
  // The login's Machine cannot run browsers now.
  assertEquals(linkBrowserOffered(read, "ovh"), false);
  // The same admission as the account cards.
  for (const admission of ["machine_browser_not_enabled", "browser_not_enabled"]) {
    assertEquals(linkBrowserOffered({ ...read, admission }, "hawk"), false);
  }
  assertEquals(linkBrowserOffered(undefined, "hawk"), false);
  // An account placed elsewhere keeps its browser there.
  const placement = (machine: string) => ({
    account_id: A,
    machine_id: machine,
    instance_id: "b-aaaaaaaaaaaaaaaaaaaaaaaa",
    profile_revision: 1,
    last_open_ms: 1,
  });
  const placed = { ...read, choose_machine: false, placements: [placement("ovh")] };
  assertEquals(linkBrowserOffered(placed, "hawk"), false);
  assertEquals(
    linkBrowserOffered({ ...placed, placements: [placement("ovh"), placement("hawk")] }, "hawk"),
    true,
  );
});

test("a link opens and navigates only this account's browser on the login's Machine", async () => {
  // The panel shows A's browser on ovh: the link for hawk opens there.
  const onOvh: BrowserRead = {
    ...openB,
    state: running(A, "b-aaaaaaaaaaaaaaaaaaaaaaaa", 7),
    placements: [],
    admission: null,
  };
  assertEquals(linkEntry(onOvh, "hawk"), "open");
  assertEquals(linkEntry(undefined, "hawk"), "open");
  assertEquals(linkEntry({ ...onOvh, machine_id: "hawk" }, "hawk"), "attach");
  const shownOnOvh = reduceBrowser(INITIAL_BROWSER_MODEL, { type: "read", read: onOvh, now: 1 });
  assertEquals(linkTarget(shownOnOvh, "hawk"), undefined);
  // Only the pinned open's answer, running on hawk, receives the link.
  const opened = reduceBrowser(shownOnOvh, {
    type: "result",
    action: "open",
    result: openA,
    now: 2,
  });
  assertEquals(linkTarget(opened, "hawk"), shownTarget(opened));
  assertEquals(linkTarget(opened, "hawk")?.instance.account_id, A);
  const starting = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "result",
    action: "open",
    result: { machine_id: "hawk", state: { ...openA.state, state: "starting" } },
    now: 2,
  });
  assertEquals(linkTarget(starting, "hawk"), undefined);

  // The calls: a tab open pinned to hawk, then a navigation of exactly that
  // instance. No route names a login, and nothing reads a code back.
  const calls: { path: string; body: unknown }[] = [];
  const api = createMachineBrowserApi((path, init) => {
    calls.push({ path, body: JSON.parse(String(init.body)) });
    return Promise.resolve(new Response(JSON.stringify(openA), { status: 200 }));
  });
  const signal = new AbortController().signal;
  await api.open(A, "d", { width: 800, height: 600, scale: 1 }, signal, "hawk");
  await api.navigate(A, "d", linkTarget(opened, "hawk")!, "https://claude.ai/oauth/x", signal);
  assertEquals(
    calls.map((call) => call.path),
    [`/api/provider-accounts/${A}/browser/open`, `/api/provider-accounts/${A}/browser/navigate`],
  );
  assertEquals(calls[0]?.body, {
    dataset: "d",
    width: 800,
    height: 600,
    scale: 1,
    machine_id: "hawk",
  });
  assertEquals(calls[1]?.body, {
    dataset: "d",
    machine_id: "hawk",
    instance: openA.state.instance,
    url: "https://claude.ai/oauth/x",
  });
  assertEquals("openSignIn" in api, false);
});

function recordingControls(account: string, target: BrowserTarget) {
  const chords: string[] = [];
  const controls: ViewerControls = {
    account,
    target,
    chord: (chord) => chords.push(chord),
    toggleKeyboard: () => {},
    paste: () => {},
  };
  return { controls, chords };
}

test("toolbar key controls act only on the viewer of the shown account, Machine and instance", () => {
  let state: AccountBrowserModel = { account: A, model: INITIAL_BROWSER_MODEL };
  state = reduceAccountBrowser(state, {
    kind: "event",
    account: A,
    event: { type: "result", action: "open", result: openA, now: 1 },
  });
  const a = modelFor(state, A);
  const aTarget = shownTarget(a)!;
  const fromA = recordingControls(A, aTarget);
  // A's own mounted viewer drives A's toolbar.
  assertEquals(controlsFor(fromA.controls, A, a), fromA.controls);
  assertEquals(chromeState(a, true, 1).navigation, true);
  // A's late controls never drive B's panel, even before B has a viewer.
  state = reduceAccountBrowser(state, { kind: "bind", account: B });
  const b0 = modelFor(state, B);
  assertEquals(controlsFor(fromA.controls, B, b0), undefined);
  assertEquals(chromeState(b0, false, 1).navigation, false);
  state = reduceAccountBrowser(state, {
    kind: "event",
    account: B,
    event: { type: "result", action: "open", result: openB, now: 2 },
  });
  const b = modelFor(state, B);
  assertEquals(controlsFor(fromA.controls, B, b), undefined);
  // Controls claiming B but bound to A's instance, or to an older
  // incarnation of B's browser, are refused too.
  assertEquals(controlsFor(recordingControls(B, aTarget).controls, B, b), undefined);
  const bTarget = shownTarget(b)!;
  const older = { ...bTarget, instance: { ...bTarget.instance, incarnation: 1 } };
  assertEquals(controlsFor(recordingControls(B, older).controls, B, b), undefined);
  const otherMachine = { ...bTarget, machine_id: "hawk" };
  assertEquals(controlsFor(recordingControls(B, otherMachine).controls, B, b), undefined);
  const fromB = recordingControls(B, bTarget);
  controlsFor(fromB.controls, B, b)?.chord("back");
  assertEquals(fromB.chords, ["back"]);
  assertEquals(fromA.chords, []);
  // Once B's viewer is gone, its controls drive nothing.
  const detached = reduceBrowser(b, { type: "detach" });
  assertEquals(controlsFor(fromB.controls, B, detached), undefined);
});

test("the address bar navigates only the running browser this account's panel shows", async () => {
  // Nothing shown or not running: no navigation, an inline reason instead.
  assertEquals(addressCommand(INITIAL_BROWSER_MODEL, B, "example.com").kind, "error");
  const hibernated = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "read",
    read: {
      machine_id: "ovh",
      state: {
        ...running(B, "", 0),
        instance: { account_id: B, instance_id: "", incarnation: 0 },
        state: "hibernated",
      },
      placements: [],
      admission: null,
    },
    now: 1,
  });
  assertEquals(addressCommand(hibernated, B, "https://example.com").kind, "error");
  assertEquals(chromeState(hibernated, false, 1).address, false);
  // B's panel with B's viewer: the command names B's Machine and instance.
  const b = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "result",
    action: "open",
    result: openB,
    now: 2,
  });
  const command = addressCommand(b, B, "example.com/path");
  assertEquals(command, {
    kind: "navigate",
    url: "https://example.com/path",
    target: shownTarget(b)!,
  });
  // The same model offered under A's identity is refused: the instance
  // belongs to B.
  assertEquals(addressCommand(b, A, "https://example.com").kind, "error");
  // A poll answering for another Machine never redirects the address bar
  // away from the mounted viewer's browser.
  const polled = reduceBrowser(b, {
    type: "read",
    read: {
      ...openA,
      state: running(B, "b-cccccccccccccccccccccccc", 9),
      placements: [],
      admission: null,
    },
    now: 3,
  });
  const after = addressCommand(polled, B, "https://example.com");
  assertEquals(after.kind === "navigate" && after.target.machine_id, "ovh");
  // While a command is in flight nothing else is sent.
  assertEquals(
    addressCommand(reduceBrowser(b, { type: "busy", action: "snapshot" }), B, "https://example.com")
      .kind,
    "error",
  );
  // The wire call names exactly B's account, Machine and instance.
  const calls: { path: string; body: unknown }[] = [];
  const api = createMachineBrowserApi((path, init) => {
    calls.push({ path, body: JSON.parse(String(init.body)) });
    return Promise.resolve(new Response(JSON.stringify(openB), { status: 200 }));
  });
  if (command.kind !== "navigate") throw new Error("expected a navigation");
  await api.navigate(B, "d", command.target, command.url, new AbortController().signal);
  assertEquals(calls, [
    {
      path: `/api/provider-accounts/${B}/browser/navigate`,
      body: {
        dataset: "d",
        machine_id: "ovh",
        instance: openB.state.instance,
        url: "https://example.com/path",
      },
    },
  ]);
});
