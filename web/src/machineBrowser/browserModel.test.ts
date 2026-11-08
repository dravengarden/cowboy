import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import type { BrowserRead, BrowserStateEvent } from "./api";
import {
  type BrowserModel,
  INITIAL_BROWSER_MODEL,
  POLL_IDLE_MS,
  POLL_STARTING_MS,
  STARTING_FAST_POLL_MS,
  autoEntry,
  looksLikeAuthorizationCode,
  openRefused,
  pinnedMachine,
  pollPeriod,
  reduceBrowser,
  resetTarget,
  targetOf,
} from "./browserModel";

const account = "11111111-2222-4333-8444-555555555555";
function event(
  state: BrowserStateEvent["state"],
  extra: Partial<BrowserStateEvent> = {},
): BrowserStateEvent {
  return {
    instance: { account_id: account, instance_id: "b-0123456789abcdef01234567", incarnation: 1 },
    state,
    observed_at_ms: 1,
    ...extra,
  };
}
const read = (state: BrowserStateEvent | null, machine = "ovh"): BrowserRead => ({
  machine_id: machine,
  state,
  placements: [],
  admission: null,
});
const shown = (machine = "ovh", incarnation = 1) => ({
  machine_id: machine,
  instance: { account_id: account, instance_id: "b-0123456789abcdef01234567", incarnation },
});

test("an open that answers detached mounts the viewer; a refused open explains itself", () => {
  let model: BrowserModel = reduceBrowser(INITIAL_BROWSER_MODEL, { type: "busy", action: "open" });
  assertEquals(model.busy, "open");
  model = reduceBrowser(model, {
    type: "result",
    action: "open",
    result: { machine_id: "ovh", state: event("detached") },
    now: 10,
  });
  assertEquals(model.busy, undefined);
  assertEquals(model.viewer, { takeover: false, generation: 1, target: shown() });
  assertEquals(model.read?.state?.state, "detached");
  const refused = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "result",
    action: "open",
    result: { machine_id: "ovh", state: event("hibernated", { reason: "capacity" }) },
    now: 10,
  });
  assertEquals(refused.viewer, undefined);
  assertEquals(refused.error.length > 0, true);
  assertEquals(refused.error.includes("_"), false);
});

test("a refused sign-in open never mounts the viewer or takes the display over", () => {
  for (const [state, reason] of [
    ["detached", "login_mismatch"],
    ["attached", "login_mismatch"],
    ["attached", "failed"],
    ["detached", "busy"],
    ["starting", "busy"],
  ] as const) {
    const answer = event(state, { reason });
    assertEquals(openRefused(answer), true, `${state}/${reason}`);
    const model = reduceBrowser(INITIAL_BROWSER_MODEL, {
      type: "result",
      action: "open",
      result: { machine_id: "ovh", state: answer },
      now: 10,
      takeover: true,
    });
    assertEquals(model.viewer, undefined, `${state}/${reason}`);
    assertEquals(model.awaitingOpen, undefined, `${state}/${reason}`);
    assertEquals(model.busy, undefined);
    assertEquals(model.error.length > 0, true);
    assertEquals(model.error.includes("_"), false);
  }
  // Without a reason the same answers are ordinary successes.
  assertEquals(openRefused(event("detached")), false);
  assertEquals(openRefused(event("hibernated", { reason: "capacity" })), false);
  const bound = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "result",
    action: "open",
    result: { machine_id: "ovh", state: event("attached") },
    now: 10,
    takeover: true,
  });
  assertEquals(bound.viewer, { takeover: true, generation: 1, target: shown() });
});

test("a starting open polls fast and attaches once the instance runs", () => {
  let model = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "result",
    action: "open",
    result: { machine_id: "ovh", state: event("starting") },
    now: 1_000,
  });
  assertEquals(model.awaitingOpen, shown());
  assertEquals(pinnedMachine(model), "ovh");
  assertEquals(pollPeriod(model, 2_000), POLL_STARTING_MS);
  assertEquals(pollPeriod(model, 1_000 + STARTING_FAST_POLL_MS), POLL_IDLE_MS);
  model = reduceBrowser(model, { type: "read", read: read(event("detached")), now: 5_000 });
  assertEquals(model.awaitingOpen, undefined);
  assertEquals(model.viewer?.takeover, false);
  assertEquals(pollPeriod(model, 5_000), undefined);
});

test("hibernated, stopping and superseded states end the viewer and keep the last state", () => {
  const attached = reduceBrowser(
    reduceBrowser(INITIAL_BROWSER_MODEL, { type: "read", read: read(event("detached")), now: 1 }),
    { type: "attach", takeover: true },
  );
  assertEquals(attached.viewer, { takeover: true, generation: 1, target: shown() });
  const held = reduceBrowser(attached, { type: "remote_clipboard", text: "code-1" });
  assertEquals(held.remoteClipboard, "code-1");
  const hibernated = reduceBrowser(held, {
    type: "stream_state",
    state: event("hibernated", { reason: "login_ended" }),
  });
  assertEquals(hibernated.viewer, undefined);
  assertEquals(hibernated.remoteClipboard, undefined);
  assertEquals(hibernated.read?.state?.state, "hibernated");
  assertEquals(hibernated.notice.includes("sign-in"), true);
  const superseded = reduceBrowser(held, {
    type: "stream_state",
    state: event("attached", { reason: "superseded" }),
  });
  assertEquals(superseded.viewer, undefined);
  const stopping = reduceBrowser(held, { type: "stream_state", state: event("stopping") });
  assertEquals(stopping.viewer, undefined);
  const running = reduceBrowser(held, {
    type: "stream_state",
    state: event("attached", { geometry: { width: 800, height: 600 } }),
  });
  assertEquals(running.viewer, attached.viewer);
  assertEquals(running.remoteClipboard, "code-1");
});

test("a closed stream clears the viewer and its clipboard with a visible message", () => {
  // Nothing shown names an instance: there is nothing to attach to.
  assertEquals(
    reduceBrowser(INITIAL_BROWSER_MODEL, { type: "attach", takeover: false }).viewer,
    undefined,
  );
  const attached = reduceBrowser(
    reduceBrowser(INITIAL_BROWSER_MODEL, { type: "read", read: read(event("detached")), now: 1 }),
    { type: "attach", takeover: false },
  );
  const ended = reduceBrowser(reduceBrowser(attached, { type: "remote_clipboard", text: "x" }), {
    type: "viewer_ended",
    message: "Sign in again to continue.",
  });
  assertEquals(ended.viewer, undefined);
  assertEquals(ended.remoteClipboard, undefined);
  assertEquals(ended.notice, "Sign in again to continue.");
  assertEquals(reduceBrowser(ended, { type: "dismiss" }).notice, "");
  const failed = reduceBrowser(reduceBrowser(attached, { type: "busy", action: "reset" }), {
    type: "failed",
    action: "reset",
    message: "nope",
  });
  assertEquals(failed.busy, undefined);
  assertEquals(failed.error, "nope");
});

test("only one whitespace-free token within the server bound reads as an authorization code", () => {
  assertEquals(looksLikeAuthorizationCode("abc123#def"), true);
  assertEquals(looksLikeAuthorizationCode("  token  "), true);
  assertEquals(looksLikeAuthorizationCode(""), false);
  assertEquals(looksLikeAuthorizationCode("two words"), false);
  assertEquals(looksLikeAuthorizationCode("a\nb"), false);
  assertEquals(looksLikeAuthorizationCode("x".repeat(4096)), true);
  assertEquals(looksLikeAuthorizationCode("x".repeat(4097)), false);
});

const noInstance = (extra: Partial<BrowserStateEvent> = {}): BrowserStateEvent => ({
  instance: { account_id: account, instance_id: "", incarnation: 0 },
  state: "hibernated",
  profile_revision: 4,
  observed_at_ms: 2,
  ...extra,
});

function finish(
  action: "snapshot" | "reset",
  state: BrowserStateEvent,
  from: BrowserModel = INITIAL_BROWSER_MODEL,
): BrowserModel {
  return reduceBrowser(reduceBrowser(from, { type: "busy", action }), {
    type: "result",
    action,
    result: { machine_id: "ovh", state },
    now: 3,
  });
}

test("a snapshot reports success only when the Machine reports no failure", () => {
  const running = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "read",
    read: read(event("detached")),
    now: 1,
  });
  const published = finish("snapshot", event("detached"), running);
  assertEquals(published.notice, "Snapshot published.");
  assertEquals(published.error, "");
  const refused = finish("snapshot", event("detached", { reason: "storage_exceeded" }), running);
  assertEquals(refused.notice, "");
  assertEquals(refused.error.includes("not published"), true);
  assertEquals(refused.error.includes("storage budget"), true);
});

test("a reset reports success only when the Machine carried it out", () => {
  const done = finish("reset", noInstance({ reason: "owner_request" }));
  assertEquals(done.notice, "The profile was reset.");
  assertEquals(done.error, "");
  assertEquals(done.read?.state?.profile_revision, 4);
  assertEquals(
    finish("reset", noInstance({ reason: "superseded" })).notice.includes("nothing"),
    true,
  );
  for (const reason of ["busy", "failed", undefined]) {
    const failed = finish("reset", noInstance(reason ? { reason } : {}));
    assertEquals(failed.notice, "", String(reason));
    assertEquals(failed.error.startsWith("The profile was not reset."), true, String(reason));
  }
});

test("a sign-in panel enters every browser through the sign-in open", () => {
  assertEquals(autoEntry(true, "detached"), "open");
  assertEquals(autoEntry(true, "hibernated"), "open");
  assertEquals(autoEntry(true, null), "open");
  assertEquals(autoEntry(true, "attached"), undefined);
  assertEquals(autoEntry(false, "detached"), "attach");
  assertEquals(autoEntry(false, undefined), "open");
  assertEquals(autoEntry(false, "starting"), undefined);
  // The sign-in open answers attached: only an explicit takeover mounts it.
  const open = (takeover: boolean): BrowserModel =>
    reduceBrowser(INITIAL_BROWSER_MODEL, {
      type: "result",
      action: "open",
      result: { machine_id: "ovh", state: event("attached", { attachment_id: "att-1" }) },
      now: 1,
      takeover,
    });
  assertEquals(open(true).viewer, { takeover: true, generation: 1, target: shown() });
  assertEquals(open(false).viewer, undefined);
});

test("a starting open attaches only to the instance it opened, on its Machine", () => {
  const starting = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "result",
    action: "open",
    result: { machine_id: "alpha", state: event("starting") },
    now: 1,
  });
  // Another Machine's newer browser of the account is never attached.
  const elsewhere = reduceBrowser(starting, {
    type: "read",
    read: read(event("detached"), "beta"),
    now: 2,
  });
  assertEquals(elsewhere.viewer, undefined);
  assertEquals(elsewhere.awaitingOpen, undefined);
  assertEquals(elsewhere.error.length > 0, true);
  // A restarted instance on the same Machine is not the one opened either.
  const restarted = reduceBrowser(starting, {
    type: "read",
    read: read(event("detached", { instance: shown("alpha", 2).instance }), "alpha"),
    now: 2,
  });
  assertEquals(restarted.viewer, undefined);
  const ready = reduceBrowser(starting, {
    type: "read",
    read: read(event("detached"), "alpha"),
    now: 2,
  });
  assertEquals(ready.viewer?.target, shown("alpha"));
});

test("the viewer target follows the displayed Machine and stream states keep it", () => {
  const shownAlpha = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "read",
    read: read(event("detached"), "alpha"),
    now: 1,
  });
  assertEquals(pinnedMachine(shownAlpha), "alpha");
  const attached = reduceBrowser(shownAlpha, { type: "attach", takeover: false });
  assertEquals(attached.viewer?.target, shown("alpha"));
  const streamed = reduceBrowser(attached, { type: "stream_state", state: event("attached") });
  assertEquals(streamed.read?.machine_id, "alpha");
  assertEquals(pinnedMachine(streamed), "alpha");
  // Once the shown browser is hibernated, reads report the preferred one.
  const stopped = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "read",
    read: read(event("hibernated"), "alpha"),
    now: 1,
  });
  assertEquals(pinnedMachine(stopped), undefined);
});

test("a reset pairs the shown Machine with that Machine's own profile revision", () => {
  const placement = (machine: string, revision: number) => ({
    account_id: account,
    machine_id: machine,
    instance_id: `b-${machine}`,
    profile_revision: revision,
    last_open_ms: 1,
  });
  const placements = [placement("alpha", 3), placement("beta", 7)];
  assertEquals(
    resetTarget({ ...read(event("detached", { profile_revision: 4 }), "beta"), placements }),
    { machine_id: "beta", revision: 4 },
  );
  // Without a state revision, the shown Machine's placement, not the first.
  assertEquals(resetTarget({ ...read(event("detached"), "beta"), placements }), {
    machine_id: "beta",
    revision: 7,
  });
  assertEquals(resetTarget({ ...read(null), machine_id: null, placements }), {
    machine_id: "alpha",
    revision: 3,
  });
  assertEquals(resetTarget({ ...read(null), machine_id: null }), undefined);
});

test("a stopping answer naming no instance offers nothing to command or attach", () => {
  const unknown = event("stopping", {
    instance: { account_id: account, instance_id: "", incarnation: 0 },
    reason: "busy",
  });
  const model = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "read",
    read: read(unknown, "alpha"),
    now: 1,
  });
  assertEquals(targetOf(model.read), undefined);
  assertEquals(reduceBrowser(model, { type: "attach", takeover: false }).viewer, undefined);
  // The panel keeps reading that Machine until its answer settles.
  assertEquals(pinnedMachine(model), "alpha");
});
