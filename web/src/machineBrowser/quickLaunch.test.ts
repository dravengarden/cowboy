import { test } from "bun:test";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { assertEquals } from "../../../tools/test-assert.ts";
import type { SubscriptionSnapshot } from "../subscriptionAccounts";
import { type BrowserStateEvent, createMachineBrowserApi } from "./api";
import { type BrowserModel, INITIAL_BROWSER_MODEL } from "./browserModel";
import { confirmText } from "./MachineBrowserActions";
import { Identity } from "./MachineBrowserAddressBar";
import { BrowserFrame, type BrowserFrameProps } from "./MachineBrowserFrame";
import { MobileBrowserToolbar } from "./MachineBrowserToolbar";
import { QuickLinkList } from "./QuickLaunch";
import { quickDisabled, quickEntry, quickLaunchPlan, quickLinks, quickStep } from "./quickLinks";
import { browserFamily, sharedHibernateNote, sharedIdentityLabel } from "./shareRules";
import { AccountBrowserEntry } from "./AccountBrowserEntry";
import { restartOn } from "./useMachineBrowser";

const LYNX = "aaaaaaaa-bbbb-8ccc-8ddd-eeeeeeeeeeee";
const PIKA = "0f0f0f0f-1e1e-8d2d-ac3c-4b4b4b4b4b4b";
const LARK = "11111111-2222-4333-8444-555555555555";
const SHARE = "21212121-3434-4565-8787-909090909090";
const GOOGLE = { share_id: SHARE, revision: 2, label: "Google", members: ["Lynx", "Pika", "Fox"] };

function render(element: React.ReactElement): string {
  const prior = Object.getOwnPropertyDescriptor(globalThis, "React");
  Object.defineProperty(globalThis, "React", { configurable: true, value: React });
  try {
    return renderToStaticMarkup(element);
  } finally {
    if (prior) Object.defineProperty(globalThis, "React", prior);
    else Reflect.deleteProperty(globalThis, "React");
  }
}

function state(value: BrowserStateEvent["state"], incarnation = 3): BrowserStateEvent {
  return {
    instance: { account_id: PIKA, instance_id: "b-1", incarnation },
    state: value,
    profile_revision: 1,
    observed_at_ms: 1,
  };
}

function model(value: BrowserStateEvent["state"] | null, viewer = false): BrowserModel {
  const target = { machine_id: "ovh", instance: state("detached").instance };
  return {
    ...INITIAL_BROWSER_MODEL,
    loaded: true,
    read: {
      machine_id: "ovh",
      state: value === null ? null : state(value),
      placements: [],
      admission: null,
    },
    ...(viewer ? { viewer: { takeover: false, generation: 1, target } } : {}),
  };
}

test("quick links put the card's own service first and Google last", () => {
  const keys = (family: string | undefined, shared = false) =>
    quickLinks(family, shared).map((link) => `${link.key}${link.emphasized ? "*" : ""}`);
  assertEquals(keys("deepseek"), ["deepseek*", "anthropic", "openai", "xai", "google"]);
  assertEquals(keys("openai"), ["openai*", "anthropic", "xai", "deepseek", "google"]);
  assertEquals(keys(undefined), ["anthropic", "openai", "xai", "deepseek", "google"]);
  // A shared browser opened from Pika: OpenAI first, Google emphasized.
  assertEquals(keys("openai", true), ["openai*", "anthropic", "xai", "deepseek", "google*"]);
  const urls = quickLinks(undefined, false).map((link) => link.url);
  assertEquals(urls, [
    "https://claude.ai/",
    "https://chatgpt.com/",
    "https://grok.com/",
    "https://chat.deepseek.com/",
    "https://accounts.google.com/",
  ]);
});

test("a quick link navigates the connected viewer, else opens, attaches or asks first", () => {
  const target = { machine_id: "ovh", instance: state("detached").instance };
  assertEquals(quickLaunchPlan(model("detached", true), target), "navigate");
  // An attachment already connecting is reused, never doubled or torn down.
  assertEquals(quickLaunchPlan(model("detached", true), undefined), "reuse");
  assertEquals(
    quickLaunchPlan({ ...model("hibernated"), awaitingOpen: target }, undefined),
    "reuse",
  );
  assertEquals(quickLaunchPlan(model("detached"), undefined), "attach");
  assertEquals(quickLaunchPlan(model("hibernated"), undefined), "open");
  assertEquals(quickLaunchPlan(model(null), undefined), "open");
  assertEquals(quickLaunchPlan(model("attached"), undefined), "takeover");
  // A never-placed account waits for the Machine chosen in the chooser.
  const choose = model(null);
  choose.read = {
    ...choose.read!,
    choose_machine: true,
    capable_machines: [{ machine_id: "ovh" }],
  };
  assertEquals(quickLaunchPlan(choose, undefined), "choose");
  // While a transition runs a tap does nothing, and the links are disabled.
  for (const busy of [
    model("starting"),
    model("stopping"),
    { ...model("hibernated"), busy: "open" as const },
  ]) {
    assertEquals(quickDisabled(busy), true);
    assertEquals(quickLaunchPlan(busy, target), "none");
  }
  // Disabled until the first read answered, while admission refuses, and
  // (against a Service with shared profiles) until the identity is known.
  assertEquals(quickDisabled(INITIAL_BROWSER_MODEL), true);
  assertEquals(quickDisabled({ ...model("hibernated"), loaded: true, read: undefined }), true);
  const refused = model("hibernated");
  refused.read = { ...refused.read!, admission: "browser_not_enabled" };
  assertEquals(quickDisabled(refused), true);
  assertEquals(quickDisabled(model("hibernated"), true), true);
  const named = model("hibernated");
  named.read = { ...named.read!, browser_identity: "own" };
  assertEquals(quickDisabled(named, true), false);
  assertEquals(quickDisabled(model("hibernated")), false);
  assertEquals(quickEntry(model("detached")), "attach");
});

test("a waiting quick link reaches only its own attachment of its own identity", () => {
  const target = { machine_id: "ovh", instance: state("detached").instance };
  const other = { machine_id: "hawk", instance: { ...target.instance, instance_id: "b-2" } };
  const viewer = { since: 5, generation: 1, target };
  const pending = {
    url: "https://claude.ai/",
    identity: `share:${SHARE}:2`,
    takeover: false,
    failures: 0,
    sinceEpoch: 4,
  };
  const now = {
    identity: `share:${SHARE}:2`,
    sheetIdentity: undefined as string | undefined,
    viewer: undefined as typeof viewer | undefined,
    connected: undefined as typeof target | undefined,
    confirming: undefined as string | undefined,
    takeoverViewer: false,
    failures: 0,
  };
  const step = (p: Parameters<typeof quickStep>[0], n: Partial<typeof now>) =>
    quickStep(p, { ...now, ...n });
  assertEquals(step(pending, {}).action, "wait");
  // The first viewer established after the tap binds the link.
  const bound = step(pending, { viewer });
  assertEquals(bound.action === "wait" && bound.pending.viewer, viewer);
  // An older viewer (before the tap) never carries it.
  assertEquals(step(pending, { viewer: { ...viewer, since: 3 } }), { action: "wait", pending });
  const held = bound.action === "wait" ? bound.pending : pending;
  // Only once that exact attachment's stream is open.
  assertEquals(step(held, { viewer, connected: target }).action, "navigate");
  assertEquals(step(held, { viewer, connected: other }).action, "wait");
  // The attachment ended or failed (viewer_ended), or another replaced it.
  assertEquals(step(held, {}).action, "drop");
  assertEquals(
    step(held, { viewer: { ...viewer, generation: 2 }, connected: target }).action,
    "drop",
  );
  // Another identity, from a read or the sheet's inventory in this render.
  assertEquals(step(held, { viewer, connected: target, identity: "own" }).action, "drop");
  assertEquals(
    step(held, { viewer, connected: target, sheetIdentity: `share:${SHARE}:3` }).action,
    "drop",
  );
  // A viewer on another Machine than the one it opens on.
  assertEquals(
    step({ ...pending, machine: "ovh" }, { viewer: { ...viewer, target: other } }).action,
    "drop",
  );
  // A failure since the tap drops it, even one repeating the error already
  // shown; failures before the tap do not.
  assertEquals(step(pending, { failures: 1 }).action, "drop");
  assertEquals(step({ ...pending, failures: 3 }, { failures: 3 }).action, "wait");
  // A takeover link waits while confirming, is carried by the confirmed
  // takeover, and is dropped when the confirmation is cancelled.
  const takeover = { ...pending, takeover: true };
  assertEquals(step(takeover, { confirming: "takeover" }).action, "wait");
  assertEquals(
    step(takeover, { takeoverViewer: true, viewer, connected: target }).action,
    "navigate",
  );
  assertEquals(step(takeover, {}).action, "drop");
});

test("the card's family comes from the Service's static table", () => {
  const snapshot = {
    dataset: { dataset_id: "d", user_id: "u" },
    registry: {
      revision: 1,
      defaults: {},
      accounts: [{ id: LARK, provider_id: "claude-code", label: "Lark", enabled: true }],
    },
    session_admission: "enabled",
    legacy_browsers: [
      {
        card: "openai",
        account_id: PIKA,
        providers: ["codex"],
        browser_enabled: true,
        available: true,
      },
      {
        card: "future",
        account_id: LYNX,
        providers: ["x"],
        browser_enabled: true,
        available: true,
      },
    ],
    browser_share_families: {
      version: 1,
      providers: { "claude-code": "anthropic" },
      cards: ["openai"],
    },
  } satisfies SubscriptionSnapshot;
  assertEquals(browserFamily(snapshot, PIKA), "openai");
  assertEquals(browserFamily(snapshot, LARK), "anthropic");
  assertEquals(browserFamily(snapshot, LYNX), undefined);
  assertEquals(browserFamily(undefined, PIKA), undefined);
});

test("shared browsers say so in the identity line, confirmations and empty page", () => {
  assertEquals(sharedIdentityLabel(GOOGLE), "Google · shared with Lynx, Pika, Fox");
  assertEquals(
    sharedHibernateNote(GOOGLE),
    "This browser is shared (Google). Hibernating it stops it for every member: Lynx, Pika, Fox.",
  );
  assertEquals(sharedHibernateNote(undefined), undefined);
  assertEquals(confirmText("reset", GOOGLE).title, "Reset the shared browser profile (Google)?");
  assertEquals(confirmText("hibernate", GOOGLE).title, "Hibernate the shared browser (Google)?");
  assertEquals(confirmText("reset", undefined).title, "Reset the browser profile?");
  assertEquals(confirmText("restart", GOOGLE).title, "Restart the shared browser (Google)?");
  assertEquals(
    sharedHibernateNote(GOOGLE, "restart"),
    "This browser is shared (Google). Restarting it stops it for every member: Lynx, Pika, Fox.",
  );
  const shared = render(
    React.createElement(Identity, {
      accountLabel: sharedIdentityLabel(GOOGLE),
      machineLabel: "OVH Matrix",
      status: "Hibernated",
      align: "center",
      shared: true,
    }),
  );
  assertEquals(shared.includes('aria-label="Shared browser"'), true);
  assertEquals(shared.includes('data-machine-browser-identity="shared"'), true);
  const own = render(
    React.createElement(Identity, {
      accountLabel: "DeepSeek · Mole",
      machineLabel: "OVH Matrix",
      status: "Hibernated",
      align: "center",
    }),
  );
  assertEquals(own.includes("Shared browser"), false);
  // The hibernated page of a shared browser is titled by its group and
  // offers the quick links under its primary action.
  const frame = (shared: typeof GOOGLE | undefined) =>
    render(
      React.createElement(BrowserFrame, {
        areaRef: { current: null },
        browser: { model: model("hibernated"), dispatch: () => {}, refresh: () => {} },
        account: { id: PIKA, label: "Pika" },
        machineLabel: "OVH Matrix",
        touch: true,
        link: false,
        pinned: false,
        retry: false,
        commands: { open: () => {} },
        enabling: false,
        onEnableBrowser: undefined,
        onConfirm: () => {},
        onControls: () => {},
        onKeyboard: () => {},
        shared,
        quick: React.createElement(QuickLinkList, {
          links: quickLinks("openai", shared !== undefined),
          disabled: false,
          onLaunch: () => {},
        }),
      } as unknown as BrowserFrameProps),
    );
  const sharedPage = frame(GOOGLE);
  assertEquals(sharedPage.includes("Shared browser · Google"), true);
  assertEquals(sharedPage.includes('data-machine-browser-title="shared"'), true);
  assertEquals(sharedPage.includes('aria-label="Open ChatGPT · Codex in this browser"'), true);
  assertEquals(
    sharedPage.indexOf('data-quick-link="openai"') < sharedPage.indexOf('data-quick-link="google"'),
    true,
  );
  const ownPage = frame(undefined);
  assertEquals(ownPage.includes("This browser is hibernating"), true);
  assertEquals(ownPage.includes("Shared browser"), false);
});

test("the phone's identity line sits below the toolbar buttons", () => {
  const html = render(
    React.createElement(MobileBrowserToolbar, {
      chrome: {
        navigation: false,
        keyboard: false,
        address: false,
        detach: false,
        hibernate: false,
        snapshot: false,
        restart: false,
        reset: false,
        paste: false,
        copy: false,
        refresh: true,
      },
      keyboardOpen: false,
      identity: React.createElement("span", { "data-identity-probe": true }, "DeepSeek · Mole"),
      onChord: () => {},
      onKeyboard: () => {},
      onClose: () => {},
      menu: React.createElement("button", { "aria-label": "More browser actions" }),
    }),
  );
  const identity = html.indexOf("data-identity-probe");
  assertEquals(identity > html.indexOf('aria-label="Close browser"'), true);
  assertEquals(identity > html.indexOf('aria-label="More browser actions"'), true);
});

test("a shared card's browser entry has its own icon, accent and name", () => {
  const snapshot = (shared: boolean): SubscriptionSnapshot => ({
    dataset: { dataset_id: "d", user_id: "u" },
    registry: { revision: 1, defaults: {}, accounts: [] },
    session_admission: "enabled",
    machine_browser: "enabled",
    legacy_browsers: [
      {
        card: "openai",
        account_id: PIKA,
        providers: ["codex"],
        browser_enabled: true,
        available: true,
      },
    ],
    browser_shares: shared
      ? [
          {
            share_id: SHARE,
            label: "Google",
            created_ms: 1,
            revision: 2,
            deleting: false,
            members: [{ member: { card: "openai" }, family: "openai", account_id: PIKA }],
          },
        ]
      : [],
    browser_share_families: { version: 1, providers: {}, cards: ["openai"] },
  });
  const entry = (shared: boolean) =>
    render(
      React.createElement(AccountBrowserEntry, { snapshot: snapshot(shared), account: "openai" }),
    );
  const shared = entry(true);
  assertEquals(shared.includes('aria-label="Shared browser: Google"'), true);
  assertEquals(shared.includes("Shared · Google"), true);
  const own = entry(false);
  assertEquals(own.includes('aria-label="Browser"'), true);
  assertEquals(own.includes("Shared"), false);
});

test("a restart's reopen is bound to the confirmed identity on the Service", async () => {
  const stopped: BrowserStateEvent = {
    instance: { account_id: PIKA, instance_id: "", incarnation: 0 },
    state: "hibernated",
    reason: "owner_request",
    profile_revision: 2,
    observed_at_ms: 1,
  };
  const bodies: unknown[] = [];
  const api = createMachineBrowserApi((path, init) => {
    if (path.endsWith("/open")) bodies.push(JSON.parse(String(init.body)));
    return Promise.resolve(
      new Response(JSON.stringify({ machine_id: "ovh", state: stopped }), { status: 200 }),
    );
  });
  const target = { machine_id: "ovh", instance: state("detached").instance };
  const identity = { expected: `share:${SHARE}:2`, observe: () => {} };
  await restartOn(
    { ...api, hibernate: () => Promise.resolve({ machine_id: "ovh", state: stopped }) },
    { account: PIKA, dataset: "d" },
    target,
    new AbortController().signal,
    (signal, machine, expected) =>
      api.open(PIKA, "d", { width: 800, height: 600, scale: 1 }, signal, machine, expected),
    identity,
  );
  assertEquals(bodies, [
    {
      dataset: "d",
      width: 800,
      height: 600,
      scale: 1,
      machine_id: "ovh",
      expected_browser: `share:${SHARE}:2`,
    },
  ]);
});

test("every failure shown advances the failure count, even with the same text", async () => {
  const { INITIAL_BROWSER_MODEL, reduceBrowser } = await import("./browserModel");
  const failed = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "failed",
    action: "open",
    message: "The Machine did not answer.",
  });
  assertEquals(failed.failures, 1);
  // A new command clears the error; its failure with the same text counts.
  const busy = reduceBrowser(failed, { type: "busy", action: "open" });
  assertEquals(busy.failures, 1);
  const again = reduceBrowser(busy, {
    type: "failed",
    action: "open",
    message: "The Machine did not answer.",
  });
  assertEquals(again.failures, 2);
  // An event that leaves the shown error alone is not a failure.
  assertEquals(reduceBrowser(again, { type: "notice", message: "hi" }).failures, 2);
});
