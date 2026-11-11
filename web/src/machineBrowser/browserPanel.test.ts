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
import { sharedHibernateNote, sharedIdentityLabel } from "./shareRules";
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
  // The hibernated page of a shared browser is titled by its group; it
  // offers no quick links (the remote new-tab page has them).
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
      } as unknown as BrowserFrameProps),
    );
  const sharedPage = frame(GOOGLE);
  assertEquals(sharedPage.includes("Shared browser · Google"), true);
  assertEquals(sharedPage.includes('data-machine-browser-title="shared"'), true);
  assertEquals(sharedPage.includes("data-quick-link"), false);
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
  // The card names its group (it is the only member, so no "with").
  assertEquals(shared.includes("Group Google<"), true);
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

test("groups are offered unused animal names and cards name their group and peers", async () => {
  const { suggestGroupName, sharedCardLabel, GROUP_NAMES } = await import("./shareRules");
  const snapshot = {
    browser_shares: [{ label: "Otter" }],
    registry: { accounts: [{ label: "badger" }] },
  } as unknown as Parameters<typeof suggestGroupName>[0];
  assertEquals(suggestGroupName(snapshot), GROUP_NAMES[2]);
  assertEquals(suggestGroupName(undefined), GROUP_NAMES[0]);
  assertEquals(
    sharedCardLabel({ share_id: "s", revision: 1, label: "Otter", members: ["A", "B"], others: ["B"] }),
    "Group Otter · with B",
  );
  assertEquals(sharedCardLabel({ share_id: "s", revision: 1, label: "Otter", members: ["A"] }), "Group Otter");
});
