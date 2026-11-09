import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { validPendingSubscription, subscriptionWriteRejected } from "../SubscriptionManagement";
import { type SubscriptionSnapshot, validSubscriptionSnapshot } from "../subscriptionAccounts";
import { type BrowserStateEvent, browserErrorText } from "./api";
import {
  type AccountBrowserModel,
  INITIAL_BROWSER_MODEL,
  modelFor,
  reduceAccountBrowser,
  shownTarget,
} from "./browserModel";
import { accountBrowserEntry } from "./entryRules";
import {
  joinBlocked,
  shareCandidates,
  shareOf,
  sharedBrowser,
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
    browser_share_families: { "claude-code": "anthropic", codex: "openai", grok: "xai" },
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
