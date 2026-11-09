import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import type { SubscriptionSnapshot } from "../subscriptionAccounts";
import {
  type BrowserRead,
  type BrowserStateEvent,
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
import {
  accountBrowserEntry,
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
