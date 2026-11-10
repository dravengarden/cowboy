import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import {
  subscriptionRejectionText,
  subscriptionWriteRejected,
  validPendingSubscription,
} from "../SubscriptionManagement";
import { type SubscriptionSnapshot, validSubscriptionSnapshot } from "../subscriptionAccounts";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  type BrowserRead,
  type BrowserReadShare,
  type BrowserStateEvent,
  browserErrorText,
  createMachineBrowserApi,
  validBrowserRead,
} from "./api";
import { BrowserShareSettings, commitDraft, sharesSupported } from "./BrowserShareSettings";
import { confirmationCurrent, confirmationFor } from "./confirmation";
import { checkRestartIdentity, resetChangesAccounts, restartOn } from "./useMachineBrowser";
import {
  type AccountBrowserModel,
  INITIAL_BROWSER_MODEL,
  modelFor,
  reduceAccountBrowser,
  reduceBrowser,
  resetTarget,
  resultRead,
  shownTarget,
} from "./browserModel";
import { accountBrowserEntry } from "./entryRules";
import {
  browserIdentityKey,
  identityBinding,
  identityRequiredFor,
  newEntryLatch,
  reconcileBinding,
  takeLatch,
  joinBlocked,
  panelShared,
  shareCandidates,
  shareOf,
  sharedBrowser,
  sharedForRead,
  sharedResetNote,
} from "./shareRules";

// Legacy card keys (UUID version 8) and named accounts (version 4).
const LYNX = "aaaaaaaa-bbbb-8ccc-8ddd-eeeeeeeeeeee";
const PIKA = "0f0f0f0f-1e1e-8d2d-ac3c-4b4b4b4b4b4b";
const FOX = "99999999-8888-8777-a666-555555555555";
const MOLE = "12345678-9abc-8def-9012-3456789abcde";
const LARK = "11111111-2222-4333-8444-555555555555";
const WORK = "66666666-7777-4888-9999-000000000000";
const SHARE = "21212121-3434-4565-8787-909090909090";

function snapshot(
  members: { member: object; family: string; account_id: string }[] = [],
): SubscriptionSnapshot {
  const card = (name: string, key: string, providers: string[]) => ({
    card: name,
    account_id: key,
    providers,
    browser_enabled: true,
    available: true,
    share_id: members.some((member) => member.account_id === key) ? SHARE : null,
  });
  return {
    dataset: { dataset_id: "d", user_id: "u" },
    registry: {
      revision: 7,
      defaults: {},
      accounts: [
        {
          id: LARK,
          provider_id: "claude-code",
          label: "Lark",
          enabled: true,
          browser_enabled: true,
        },
        { id: WORK, provider_id: "codex", label: "Work", enabled: true, browser_enabled: true },
      ],
    },
    display_preferences: {
      scope: "subscriptions",
      revision: 1,
      values: {
        legacy_labels: {
          "claude-code": "Claude · Lynx",
          codex: "OpenAI · Pika",
          grok: "Grok · Fox",
        },
      },
    },
    session_admission: "enabled",
    machine_browser: "enabled",
    legacy_browsers: [
      card("anthropic", LYNX, ["claude-code"]),
      card("deepseek", MOLE, ["claude-deepseek"]),
      card("openai", PIKA, ["codex"]),
      card("xai", FOX, ["grok"]),
    ],
    browser_shares: [
      {
        share_id: SHARE,
        label: "Google",
        created_ms: 1,
        revision: 2,
        deleting: false,
        members: members as never,
      },
    ],
    browser_share_families: {
      version: 1,
      providers: { "claude-code": "anthropic", codex: "openai", grok: "xai" },
      cards: ["anthropic", "deepseek", "openai", "xai"],
    },
  };
}

const google = [
  { member: { card: "anthropic" }, family: "anthropic", account_id: LYNX },
  { member: { card: "openai" }, family: "openai", account_id: PIKA },
  { member: { card: "xai" }, family: "xai", account_id: FOX },
];

test("a group offers each card, refusing a second account of a family with the reason", () => {
  const accounts = snapshot(google.slice(0, 1));
  assertEquals(validSubscriptionSnapshot(accounts, "u"), true);
  const share = accounts.browser_shares![0]!;
  const reason = (id: string) => {
    const candidate = shareCandidates(accounts).find((entry) => entry.browserId === id)!;
    return joinBlocked(share, candidate);
  };
  // Lark is another Claude account: never linkable with Lynx.
  assertEquals(reason(LARK), "Another Claude account is already in this group.");
  // Pika (OpenAI) and Fox (Grok) may join next to Lynx.
  assertEquals(reason(PIKA), undefined);
  assertEquals(reason(FOX), undefined);
  // Once Pika is in, the named OpenAI account is refused too.
  const full = snapshot(google);
  const work = shareCandidates(full).find((entry) => entry.browserId === WORK)!;
  assertEquals(
    joinBlocked(full.browser_shares![0]!, work),
    "Another OpenAI account is already in this group.",
  );
  // A Provider without a family never joins.
  const unknown = { ...work, family: undefined };
  assertEquals(
    joinBlocked(full.browser_shares![0]!, unknown),
    "This account has no known family, so it cannot share a browser.",
  );
  assertEquals(joinBlocked({ ...share, deleting: true }, work), "This group is being deleted.");
});

test("a shared card's entry names its group and the reset names every member", () => {
  const accounts = snapshot(google);
  for (const [card, key] of [
    ["anthropic", LYNX],
    ["openai", PIKA],
    ["xai", FOX],
  ] as const) {
    const entry = accountBrowserEntry(accounts, card);
    // The card still opens through its own identity; the Service resolves it.
    assertEquals(entry.kind === "ready" && entry.account.id === key, true);
    assertEquals(shareOf(accounts, key)?.share_id, SHARE);
    assertEquals(sharedBrowser(accounts, key), {
      share_id: SHARE,
      revision: 2,
      label: "Google",
      members: ["Claude · Lynx", "OpenAI · Pika", "Grok · Fox"],
    });
  }
  // Mole and Lark are not members: their own browsers, no share.
  assertEquals(sharedBrowser(accounts, MOLE), undefined);
  assertEquals(sharedBrowser(accounts, LARK), undefined);
  assertEquals(
    sharedResetNote(sharedBrowser(accounts, PIKA)),
    "This profile is shared (Google). Resetting it signs out every member: Claude · Lynx, OpenAI · Pika, Grok · Fox.",
  );
  assertEquals(sharedResetNote(undefined), undefined);
  // A share being deleted is no longer any card's browser.
  const deleting = snapshot(google);
  deleting.browser_shares![0]!.deleting = true;
  assertEquals(sharedBrowser(deleting, PIKA), undefined);
});

test("two members of one share keep separate panels named by their own cards", () => {
  // The Service reports the share's instance under each member's own id.
  const running = (account: string): BrowserStateEvent => ({
    instance: { account_id: account, instance_id: "b-ssssssssssssssssssssssss", incarnation: 4 },
    state: "detached",
    profile_revision: 1,
    observed_at_ms: 1,
  });
  let state: AccountBrowserModel = { account: LYNX, model: INITIAL_BROWSER_MODEL };
  state = reduceAccountBrowser(state, {
    kind: "event",
    account: LYNX,
    event: {
      type: "result",
      action: "open",
      result: { machine_id: "ovh", state: running(LYNX) },
      now: 1,
    },
  });
  assertEquals(shownTarget(modelFor(state, LYNX))?.instance.account_id, LYNX);
  // Pika's panel starts from nothing and never adopts Lynx's answers.
  state = reduceAccountBrowser(state, { kind: "bind", account: PIKA });
  assertEquals(modelFor(state, PIKA), INITIAL_BROWSER_MODEL);
  state = reduceAccountBrowser(state, {
    kind: "event",
    account: LYNX,
    event: {
      type: "result",
      action: "open",
      result: { machine_id: "ovh", state: running(LYNX) },
      now: 2,
    },
  });
  assertEquals(modelFor(state, PIKA), INITIAL_BROWSER_MODEL);
  state = reduceAccountBrowser(state, {
    kind: "event",
    account: PIKA,
    event: {
      type: "result",
      action: "open",
      result: { machine_id: "ovh", state: running(PIKA) },
      now: 3,
    },
  });
  assertEquals(shownTarget(modelFor(state, PIKA))?.instance.account_id, PIKA);
});

test("share changes journal exactly their fields and their refusals are final", () => {
  const pending = (change: object) => ({
    operation: "op",
    dataset: "d",
    expected_revision: 7,
    change,
  });
  for (const change of [
    { type: "create_browser_share", label: "Google" },
    { type: "rename_browser_share", share_id: SHARE, label: "Google" },
    { type: "delete_browser_share", share_id: SHARE },
    { type: "set_browser_share", member: { card: "openai" }, share_id: SHARE },
    { type: "set_browser_share", member: { account_id: LARK }, share_id: null },
  ]) {
    assertEquals(validPendingSubscription(pending(change)), true);
  }
  for (const change of [
    { type: "create_browser_share", label: " " },
    { type: "delete_browser_share", share_id: SHARE, member: { card: "x" } },
    { type: "set_browser_share", member: { card: "openai", account_id: LARK }, share_id: SHARE },
    { type: "set_browser_share", member: {}, share_id: SHARE },
    { type: "set_browser_share", member: { card: "openai" } },
  ]) {
    assertEquals(validPendingSubscription(pending(change)), false);
  }
  assertEquals(subscriptionWriteRejected(422, "browser_share_family_conflict"), true);
  assertEquals(subscriptionWriteRejected(503, "browser_share_family_conflict"), false);
  assertEquals(
    browserErrorText("browser_share_family_conflict"),
    "This shared browser would hold two accounts of one kind, so it was not opened.",
  );
  // Malformed share listings are rejected as a whole.
  const accounts = snapshot(google);
  assertEquals(
    validSubscriptionSnapshot(
      {
        ...accounts,
        browser_shares: [{ ...accounts.browser_shares![0]!, members: [{ family: "x" }] }],
      },
      "u",
    ),
    false,
  );
});

test("a family that used the group's profile stays bound until a reset", () => {
  // Lynx joined and left; the profile may hold Lynx's Claude cookies.
  const accounts = snapshot([]);
  const share = {
    ...accounts.browser_shares![0]!,
    bound: [{ family: "anthropic", account_id: LYNX }],
  };
  assertEquals(validSubscriptionSnapshot({ ...accounts, browser_shares: [share] }, "u"), true);
  const candidate = (id: string) =>
    shareCandidates(accounts).find((entry) => entry.browserId === id)!;
  assertEquals(
    joinBlocked(share, candidate(LARK)),
    "Another Claude account used this group's profile. Reset the profile before adding this one.",
  );
  // Lynx itself may come back; other families are unaffected.
  assertEquals(joinBlocked(share, candidate(LYNX)), undefined);
  assertEquals(joinBlocked(share, candidate(PIKA)), undefined);
});

test("every opener re-binds its panel when the card moves between groups", () => {
  const before = sharedBrowser(snapshot(google), PIKA);
  const moved = snapshot(google);
  moved.browser_shares![0]!.revision = 3;
  const after = sharedBrowser(moved, PIKA);
  assertEquals(browserIdentityKey(before), `share:${SHARE}:2`);
  assertEquals(browserIdentityKey(after) === browserIdentityKey(before), false);
  assertEquals(browserIdentityKey(sharedBrowser(snapshot([]), PIKA)), "own");
  // The panel's own read is the one source of what a reset deletes, named
  // from the inventory (a sign-in link opener included).
  const read: BrowserReadShare = {
    share_id: SHARE,
    label: "Google",
    revision: 2,
    members: google.map(({ family, account_id }) => ({ family, account_id })),
  };
  assertEquals(sharedForRead(read, snapshot(google))?.members, [
    "Claude · Lynx",
    "OpenAI · Pika",
    "Grok · Fox",
  ]);
  assertEquals(sharedForRead(read, undefined)?.members, ["Claude", "OpenAI", "Grok"]);
  assertEquals(sharedForRead(null, snapshot(google)), undefined);
});

test("a reset carries the browser identity it was confirmed for", async () => {
  const read: BrowserRead = {
    machine_id: "ovh",
    state: {
      instance: { account_id: PIKA, instance_id: "b-1", incarnation: 1 },
      state: "hibernated",
      profile_revision: 4,
      observed_at_ms: 1,
    },
    placements: [],
    admission: null,
    browser_identity: `share:${SHARE}:2`,
    share: { share_id: SHARE, label: "Google", revision: 2, members: [] },
  };
  assertEquals(validBrowserRead(read), true);
  assertEquals(validBrowserRead({ ...read, share: { share_id: SHARE } }), false);
  assertEquals(validBrowserRead({ ...read, browser_identity: 3 }), false);
  const model = reduceBrowser(INITIAL_BROWSER_MODEL, { type: "read", read, now: 1 });
  assertEquals(resetTarget(read), { machine_id: "ovh", revision: 4, identity: `share:${SHARE}:2` });
  const asked = confirmationFor("reset", model)!;
  assertEquals(confirmationCurrent(asked, model), true);
  // The card left the group meanwhile: the open confirmation closes.
  const left = reduceBrowser(model, {
    type: "read",
    read: { ...read, browser_identity: "own", share: null },
    now: 2,
  });
  assertEquals(confirmationCurrent(asked, left), false);
  // The wire call names the identity; the Service refuses a stale one.
  const calls: unknown[] = [];
  const api = createMachineBrowserApi((_path, init) => {
    calls.push(JSON.parse(String(init.body)));
    return Promise.resolve(new Response(JSON.stringify({ machine_id: "ovh", state: read.state })));
  });
  const signal = new AbortController().signal;
  await api.reset(PIKA, "d", "ovh", 4, signal, `share:${SHARE}:2`);
  await api.reset(PIKA, "d", "ovh", 4, signal);
  assertEquals(calls, [
    {
      dataset: "d",
      machine_id: "ovh",
      expected_profile_revision: 4,
      expected_browser: `share:${SHARE}:2`,
    },
    { dataset: "d", machine_id: "ovh", expected_profile_revision: 4 },
  ]);
  assertEquals(
    browserErrorText("browser_identity_changed").startsWith("This card's browser changed"),
    true,
  );
});

test("a refused or conflicting share write keeps the owner's draft", async () => {
  let cleared = 0;
  const change = { type: "create_browser_share", label: "Google" } as const;
  assertEquals(
    await commitDraft(
      () => Promise.resolve(false),
      change,
      () => cleared++,
    ),
    false,
  );
  assertEquals(cleared, 0);
  assertEquals(
    await commitDraft(
      () => Promise.resolve(true),
      change,
      () => cleared++,
    ),
    true,
  );
  assertEquals(cleared, 1);
});

test("group candidates have contextual names and explain a disabled family", () => {
  const html = renderSettings(snapshot(google.slice(0, 1)));
  // Lark next to Lynx: disabled, named for the card and group, explained.
  const lark = /<button[^>]*aria-label="Add Claude · Lark to Google"[^>]*>/.exec(html)?.[0] ?? "";
  assertEquals(lark.includes("disabled"), true);
  const described = /aria-describedby="([^"]+)"/.exec(lark)?.[1];
  assertEquals(described !== undefined && html.includes(`id="${described}"`), true);
  assertEquals(html.includes("Another Claude account is already in this group."), true);
  const pika = /<button[^>]*aria-label="Add OpenAI · Pika to Google"[^>]*>/.exec(html)?.[0] ?? "";
  assertEquals(pika.length > 0 && !pika.includes("disabled"), true);
  assertEquals(html.includes('aria-label="Claude · Lynx in Google"'), true);
});

function renderSettings(accounts: SubscriptionSnapshot): string {
  const prior = Object.getOwnPropertyDescriptor(globalThis, "React");
  Object.defineProperty(globalThis, "React", { configurable: true, value: React });
  try {
    return renderToStaticMarkup(
      React.createElement(BrowserShareSettings, {
        snapshot: accounts,
        disabled: false,
        save: () => Promise.resolve(false),
      }),
    );
  } finally {
    if (prior) Object.defineProperty(globalThis, "React", prior);
    else Reflect.deleteProperty(globalThis, "React");
  }
}

test("a Service with shared profiles resets only an identity-bearing read", () => {
  // The first read failed; an open answer was merged into a read that names
  // no browser identity and no share.
  const synthesized: BrowserRead = {
    machine_id: "ovh",
    state: {
      instance: { account_id: PIKA, instance_id: "b-1", incarnation: 1 },
      state: "hibernated",
      profile_revision: 4,
      observed_at_ms: 1,
    },
    placements: [],
    admission: null,
  };
  assertEquals(resetTarget(synthesized, true), undefined);
  const model = reduceBrowser(INITIAL_BROWSER_MODEL, { type: "read", read: synthesized, now: 1 });
  assertEquals(confirmationFor("reset", model, true), undefined);
  // An older Service without shared profiles keeps resetting as before.
  assertEquals(resetTarget(synthesized), { machine_id: "ovh", revision: 4 });
  // The inventory's warning stays until the Service names the identity.
  const inventory = sharedBrowser(snapshot(google), PIKA);
  assertEquals(panelShared(synthesized, snapshot(google), inventory), inventory);
  assertEquals(panelShared(undefined, snapshot(google), inventory), inventory);
  const authoritative = {
    ...synthesized,
    browser_identity: `share:${SHARE}:2`,
    share: { share_id: SHARE, label: "Google", revision: 2, members: [] },
  };
  assertEquals(panelShared(authoritative, snapshot(google), inventory)?.members, []);
  assertEquals(
    panelShared(
      { ...authoritative, browser_identity: "own", share: null },
      snapshot(google),
      inventory,
    ),
    undefined,
  );
  assertEquals(resetTarget(authoritative, true)?.identity, `share:${SHARE}:2`);
});

test("a read naming another browser identity re-binds the panel", () => {
  assertEquals(identityBinding(undefined, `share:${SHARE}:2`), "bind");
  assertEquals(identityBinding(`share:${SHARE}:2`, `share:${SHARE}:2`), "keep");
  // A synthesized read without an identity changes nothing.
  assertEquals(identityBinding(`share:${SHARE}:2`, undefined), "keep");
  // The card left the group (or moved): model, viewer and every open
  // confirmation (reset, takeover, hibernate) are dropped by a remount.
  assertEquals(identityBinding(`share:${SHARE}:2`, "own"), "rebind");
  assertEquals(identityBinding(`share:${SHARE}:2`, `share:${SHARE}:3`), "rebind");
});

test("a family bound to a departed member is a final, explained refusal", () => {
  for (const code of [
    "browser_share_family_bound",
    "browser_share_family_drift",
    "browser_share_changed",
    "browser_share_reset_pending",
  ]) {
    assertEquals(subscriptionWriteRejected(422, code), true);
  }
  assertEquals(
    subscriptionRejectionText("browser_share_family_bound").includes("Reset the group's profile"),
    true,
  );
  assertEquals(
    subscriptionRejectionText("account_capacity"),
    "Change rejected (account_capacity). Review the list before trying again.",
  );
});

test("identity-less resets wait for an inventory proving shares unsupported", () => {
  // Loading or failed inventory: a reset must name the identity.
  assertEquals(identityRequiredFor(undefined), true);
  assertEquals(identityRequiredFor(snapshot(google)), true);
  const older = snapshot(google);
  delete older.browser_shares;
  assertEquals(identityRequiredFor(older), false);
});

test("a move to the card's own browser is observed even from a lower incarnation", () => {
  // The panel views the share's instance at incarnation 4.
  const model = {
    ...INITIAL_BROWSER_MODEL,
    loaded: true,
    viewer: {
      takeover: false,
      generation: 1,
      target: {
        machine_id: "ovh",
        instance: { account_id: PIKA, instance_id: "b-share", incarnation: 4 },
      },
    },
  };
  const own: BrowserRead = {
    machine_id: "ovh",
    state: {
      instance: { account_id: PIKA, instance_id: "b-own", incarnation: 1 },
      state: "detached",
      profile_revision: 1,
      observed_at_ms: 2,
    },
    placements: [],
    admission: null,
    browser_identity: "own",
    share: null,
  };
  const next = reduceBrowser(model, { type: "read", read: own, now: 2 });
  // The read itself is stale for the viewer and is not applied ...
  assertEquals(next.read, model.read);
  // ... but its identity is observed, so the panel re-binds.
  assertEquals(next.observedIdentity, "own");
  assertEquals(identityBinding(`share:${SHARE}:2`, next.observedIdentity), "rebind");
});

test("an older Service without shared profiles offers no share mutation", () => {
  const older = snapshot(google);
  delete older.browser_shares;
  assertEquals(sharesSupported(older), false);
  assertEquals(renderSettings(older), "");
  assertEquals(sharesSupported(snapshot([])), true);
  // A card outside the Service's family table never joins.
  const unknown = snapshot([]);
  unknown.browser_share_families = { version: 1, providers: {}, cards: [] };
  const lynx = shareCandidates(unknown).find((entry) => entry.browserId === LYNX)!;
  assertEquals(lynx.family, undefined);
  assertEquals(
    joinBlocked(unknown.browser_shares![0]!, lynx),
    "This account has no known family, so it cannot share a browser.",
  );
});

test("a re-bound panel never repeats the sheet's link navigation", () => {
  // The latch lives above the keyed panel: the first panel consumes it,
  // the one mounted after a re-bind finds it taken.
  const latch = newEntryLatch();
  assertEquals(takeLatch(latch, "linked"), true);
  assertEquals(takeLatch(latch, "linked"), false);
  assertEquals(takeLatch(latch, "autoOpened"), true);
  assertEquals(takeLatch(latch, "autoOpened"), false);
});

test("a lifecycle answer naming another identity re-binds and holds Reset", () => {
  const read: BrowserRead = {
    machine_id: "ovh",
    state: {
      instance: { account_id: PIKA, instance_id: "b-1", incarnation: 3 },
      state: "detached",
      profile_revision: 2,
      observed_at_ms: 1,
    },
    placements: [],
    admission: null,
    browser_identity: `share:${SHARE}:2`,
    share: { share_id: SHARE, label: "Google", revision: 2, members: [] },
  };
  const result = {
    machine_id: "ovh",
    state: { ...read.state!, state: "hibernated" as const },
    browser_identity: "own",
  };
  // A matching identity merges as before.
  assertEquals(
    resultRead(read, { ...result, browser_identity: `share:${SHARE}:2` }).browser_identity,
    `share:${SHARE}:2`,
  );
  // Another identity: the merged read names none, so Reset waits for a
  // state read that names it again ...
  const merged = resultRead(read, result);
  assertEquals(merged.browser_identity, undefined);
  assertEquals(merged.share, undefined);
  assertEquals(resetTarget(merged, true), undefined);
  // ... and the panel observes the new identity and re-binds.
  const model = reduceBrowser(
    reduceBrowser(INITIAL_BROWSER_MODEL, { type: "read", read, now: 1 }),
    { type: "result", action: "hibernate", result, now: 2 },
  );
  assertEquals(model.observedIdentity, "own");
  assertEquals(identityBinding(`share:${SHARE}:2`, model.observedIdentity), "rebind");
  assertEquals(resetTarget(model.read, true), undefined);
});

test("a restart continuation ends with its panel", async () => {
  const lifetime = new AbortController();
  const target = {
    machine_id: "ovh",
    instance: { account_id: PIKA, instance_id: "b-1", incarnation: 3 },
  };
  const stopped: BrowserStateEvent = {
    instance: { account_id: PIKA, instance_id: "", incarnation: 0 },
    state: "hibernated",
    reason: "owner_request",
    profile_revision: 2,
    observed_at_ms: 1,
  };
  let reopened = 0;
  const api = {
    hibernate: () => {
      // The card moves to another browser identity: the panel re-binds.
      lifetime.abort();
      return Promise.resolve({ machine_id: "ovh", state: stopped });
    },
    read: () =>
      Promise.resolve({ machine_id: "ovh", state: stopped, placements: [], admission: null }),
  } as unknown as Parameters<typeof restartOn>[0];
  let refused = false;
  try {
    await restartOn(api, { account: PIKA, dataset: "d" }, target, lifetime.signal, () => {
      reopened += 1;
      return Promise.resolve({ machine_id: "ovh", state: stopped });
    });
  } catch {
    refused = true;
  }
  assertEquals(refused, true);
  assertEquals(reopened, 0);
});

test("inventory and reads feed one binding: agreement never remounts", () => {
  const share = `share:${SHARE}:2`;
  // The first read binds the panel without a remount.
  let binding = reconcileBinding({ identity: undefined, epoch: 0 }, share);
  assertEquals(binding, { identity: share, epoch: 0 });
  // The inventory later confirms the identity the read bound: no remount.
  assertEquals(reconcileBinding(binding, share) === binding, true);
  assertEquals(reconcileBinding(binding, undefined) === binding, true);
  // Either source naming another identity re-binds exactly once.
  binding = reconcileBinding(binding, "own");
  assertEquals(binding, { identity: "own", epoch: 1 });
  assertEquals(reconcileBinding(binding, "own"), binding);
});

test("a restart stops at an intermediate answer naming another identity", async () => {
  const observed: string[] = [];
  const identity = { expected: `share:${SHARE}:2`, observe: (id: string) => observed.push(id) };
  checkRestartIdentity(identity, { browser_identity: `share:${SHARE}:2` });
  checkRestartIdentity(identity, {});
  checkRestartIdentity({ ...identity, expected: undefined }, { browser_identity: "own" });
  assertEquals(observed, []);
  const target = {
    machine_id: "ovh",
    instance: { account_id: PIKA, instance_id: "b-1", incarnation: 3 },
  };
  const stopped: BrowserStateEvent = {
    instance: { account_id: PIKA, instance_id: "", incarnation: 0 },
    state: "hibernated",
    reason: "owner_request",
    profile_revision: 2,
    observed_at_ms: 1,
  };
  let reopened = 0;
  const api = {
    // The hibernate answer already names the card's own browser.
    hibernate: () =>
      Promise.resolve({ machine_id: "ovh", state: stopped, browser_identity: "own" }),
    read: () =>
      Promise.resolve({ machine_id: "ovh", state: stopped, placements: [], admission: null }),
  } as unknown as Parameters<typeof restartOn>[0];
  let code = "";
  try {
    await restartOn(
      api,
      { account: PIKA, dataset: "d" },
      target,
      new AbortController().signal,
      () => {
        reopened += 1;
        return Promise.resolve({ machine_id: "ovh", state: stopped });
      },
      identity,
    );
  } catch (cause) {
    code = (cause as { code?: string }).code ?? "";
  }
  assertEquals(code, "browser_identity_changed");
  assertEquals(reopened, 0);
  assertEquals(observed, ["own"]);
  // The panel observes it and re-binds.
  const model = reduceBrowser(INITIAL_BROWSER_MODEL, { type: "identity", identity: "own" });
  assertEquals(model.observedIdentity, "own");
});

test("only a carried-out shared reset re-reads the account inventory", () => {
  const answer = (reason: BrowserStateEvent["reason"]) => ({
    machine_id: "ovh",
    state: {
      instance: { account_id: PIKA, instance_id: "", incarnation: 0 },
      state: "hibernated" as const,
      reason,
      profile_revision: 3,
      observed_at_ms: 1,
    },
  });
  assertEquals(resetChangesAccounts(`share:${SHARE}:2`, answer("owner_request")), true);
  assertEquals(resetChangesAccounts("own", answer("owner_request")), false);
  assertEquals(resetChangesAccounts(undefined, answer("owner_request")), false);
  for (const reason of ["superseded", "busy", "failed"] as const) {
    assertEquals(resetChangesAccounts(`share:${SHARE}:2`, answer(reason)), false);
  }
});
