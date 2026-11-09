import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { type BrowserRead, type BrowserStateEvent, MachineBrowserError } from "./api";
import {
  type BrowserModel,
  INITIAL_BROWSER_MODEL,
  POLL_IDLE_MS,
  POLL_STARTING_MS,
  STARTING_FAST_POLL_MS,
  autoEntry,
  openRefused,
  pinnedMachine,
  CHANGED_BEFORE_CONFIRM,
  pollPeriod,
  RESTART_CONFIRM_ATTEMPTS,
  confirmStopped,
  readTicket,
  reduceBrowser,
  resetTarget,
  shownTarget,
  stoppedFor,
  targetOf,
} from "./browserModel";
import { confirmationCurrent, confirmationFor } from "./confirmation";

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
  assertEquals(model.viewer, { takeover: false, generation: 1, target: shown(), since: 1 });
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

test("a refused open never mounts the viewer", () => {
  for (const [state, reason] of [
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
  assertEquals(attached.viewer, { takeover: true, generation: 1, target: shown(), since: 1 });
  const held = reduceBrowser(attached, { type: "remote_clipboard", text: "code-1" });
  assertEquals(held.remoteClipboard, "code-1");
  const hibernated = reduceBrowser(held, {
    type: "stream_state",
    state: event("hibernated", { reason: "grace" }),
  });
  assertEquals(hibernated.viewer, undefined);
  assertEquals(hibernated.remoteClipboard, undefined);
  assertEquals(hibernated.read?.state?.state, "hibernated");
  assertEquals(hibernated.notice.includes("Nobody was attached"), true);
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

test("a panel opens a stopped browser and attaches to a detached one", () => {
  assertEquals(autoEntry("detached"), "attach");
  assertEquals(autoEntry("hibernated"), "open");
  assertEquals(autoEntry(null), "open");
  assertEquals(autoEntry(undefined), "open");
  assertEquals(autoEntry("attached"), undefined);
  assertEquals(autoEntry("starting"), undefined);
  // An open answering attached never takes the display over by itself.
  const open = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "result",
    action: "open",
    result: { machine_id: "ovh", state: event("attached", { attachment_id: "att-1" }) },
    now: 1,
  });
  assertEquals(open.viewer, undefined);
});

test("a refused navigation reports its reason and keeps the viewer", () => {
  const viewing = reduceBrowser(
    reduceBrowser(INITIAL_BROWSER_MODEL, { type: "read", read: read(event("detached")), now: 1 }),
    { type: "attach", takeover: false },
  );
  const refused = reduceBrowser(reduceBrowser(viewing, { type: "busy", action: "navigate" }), {
    type: "result",
    action: "navigate",
    result: { machine_id: "ovh", state: event("attached", { reason: "failed" }) },
    now: 2,
  });
  assertEquals(refused.busy, undefined);
  assertEquals(refused.error.startsWith("The page was not opened."), true);
  assertEquals(refused.viewer?.target, shown());
  const landed = reduceBrowser(reduceBrowser(viewing, { type: "busy", action: "navigate" }), {
    type: "result",
    action: "navigate",
    result: { machine_id: "ovh", state: event("attached") },
    now: 2,
  });
  assertEquals(landed.error, "");
  assertEquals(landed.viewer?.target, shown());
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

/** A sign-in open on alpha mounted the viewer. */
function viewerOnAlpha(): BrowserModel {
  return reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "result",
    action: "open",
    result: { machine_id: "alpha", state: event("detached") },
    now: 1,
  });
}

test("a read for another Machine never replaces the mounted viewer's browser", () => {
  const mounted = viewerOnAlpha();
  assertEquals(mounted.viewer?.target, shown("alpha"));
  // A poll pinned to beta finishes after the open mounted the viewer.
  const late = reduceBrowser(mounted, {
    type: "read",
    read: read(event("detached", { observed_at_ms: 9 }), "beta"),
    now: 2,
  });
  assertEquals(late.read, mounted.read);
  assertEquals(late.viewer, mounted.viewer);
  assertEquals(shownTarget(late), shown("alpha"));
  // An older incarnation on the viewer's own Machine is stale too.
  const newer = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "result",
    action: "open",
    result: {
      machine_id: "alpha",
      state: event("detached", { instance: shown("alpha", 3).instance }),
    },
    now: 1,
  });
  const older = reduceBrowser(newer, {
    type: "read",
    read: read(event("detached"), "alpha"),
    now: 2,
  });
  assertEquals(older.read, newer.read);
  // A newer instance on the viewer's Machine replaced its browser: the
  // viewer ends rather than commanding the new instance by mistake.
  const replaced = reduceBrowser(mounted, {
    type: "read",
    read: read(event("detached", { instance: shown("alpha", 2).instance }), "alpha"),
    now: 3,
  });
  assertEquals(replaced.viewer, undefined);
  assertEquals(targetOf(replaced.read), shown("alpha", 2));
});

test("a read ticket is obsolete once a newer read starts or the pin moves", () => {
  const generations = { current: 0 };
  let model: BrowserModel = { ...INITIAL_BROWSER_MODEL, read: read(event("detached"), "beta") };
  const first = readTicket(generations, () => model);
  assertEquals(first.pinned, "beta");
  assertEquals(first.current(), true);
  // An open on alpha mounts the viewer before the beta poll answers.
  model = viewerOnAlpha();
  assertEquals(first.current(), false);
  const second = readTicket(generations, () => model);
  assertEquals(second.pinned, "alpha");
  const third = readTicket(generations, () => model);
  assertEquals(second.current(), false);
  assertEquals(third.current(), true);
});

test("a confirmation acts on the browser shown when it opened, or not at all", () => {
  const placement = (machine: string, revision: number) => ({
    account_id: account,
    machine_id: machine,
    instance_id: `b-${machine}`,
    profile_revision: revision,
    last_open_ms: 1,
  });
  const onAlpha: BrowserModel = {
    ...INITIAL_BROWSER_MODEL,
    loaded: true,
    read: { ...read(null, "alpha"), placements: [placement("alpha", 3), placement("beta", 7)] },
  };
  const reset = confirmationFor("reset", onAlpha);
  assertEquals(reset, { kind: "reset", target: { machine_id: "alpha", revision: 3 } });
  if (!reset) throw new Error("no reset target");
  assertEquals(confirmationCurrent(reset, onAlpha), true);
  // Polling switches the panel to a running browser on beta: the reset
  // confirmation no longer applies and must close without deleting beta.
  const onBeta = reduceBrowser(onAlpha, {
    type: "read",
    read: { ...read(event("detached", { profile_revision: 7 }), "beta"), placements: [] },
    now: 2,
  });
  assertEquals(confirmationCurrent(reset, onBeta), false);
  // A changed revision on the same Machine is a different reset too.
  const bumped = reduceBrowser(onAlpha, {
    type: "read",
    read: { ...read(event("hibernated", { profile_revision: 4 }), "alpha"), placements: [] },
    now: 2,
  });
  assertEquals(confirmationCurrent(reset, bumped), false);
  // Instance confirmations capture the viewer's browser while one is mounted.
  const hibernate = confirmationFor("hibernate", viewerOnAlpha());
  assertEquals(hibernate, { kind: "hibernate", target: shown("alpha") });
  if (!hibernate) throw new Error("no hibernate target");
  assertEquals(confirmationCurrent(hibernate, onBeta), false);
  assertEquals(confirmationFor("restart", onAlpha), undefined);
});

test("a confirmed takeover never attaches to a browser that replaced the confirmed one", () => {
  const attachedAlpha = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "read",
    read: read(event("attached"), "alpha"),
    now: 1,
  });
  const onBeta = reduceBrowser(attachedAlpha, {
    type: "read",
    read: read(event("attached"), "beta"),
    now: 2,
  });
  const refused = reduceBrowser(onBeta, {
    type: "attach",
    takeover: true,
    target: shown("alpha"),
  });
  assertEquals(refused.viewer, undefined);
  assertEquals(refused.error, CHANGED_BEFORE_CONFIRM);
  const taken = reduceBrowser(attachedAlpha, {
    type: "attach",
    takeover: true,
    target: shown("alpha"),
  });
  assertEquals(taken.viewer?.target, shown("alpha"));
});

test("a read issued before a lifecycle transition never ends the viewer it established", () => {
  const generations = { current: 0 };
  // Restart: the old viewer ended while the browser was stopping.
  let model: BrowserModel = reduceBrowser(INITIAL_BROWSER_MODEL, {
    type: "read",
    read: read(event("stopping")),
    now: 1,
  });
  const whileStopping = readTicket(generations, () => model);
  assertEquals(whileStopping.pinned, "ovh");
  // The replacement opens on the same Machine before that read answers.
  model = reduceBrowser(model, {
    type: "result",
    action: "open",
    result: { machine_id: "ovh", state: event("detached", { instance: shown("ovh", 2).instance }) },
    now: 2,
  });
  assertEquals(model.viewer?.target, shown("ovh", 2));
  assertEquals(whileStopping.current(), false);
  // Its incarnation-less "hibernated" arrives anyway (a queued dispatch).
  const late = read(
    event("hibernated", { instance: { ...shown().instance, instance_id: "", incarnation: 0 } }),
  );
  const after = reduceBrowser(model, {
    type: "read",
    read: late,
    now: 3,
    issued: whileStopping.epoch,
  });
  assertEquals(after.viewer?.target, shown("ovh", 2));
  const failed = reduceBrowser(model, {
    type: "read_failed",
    message: "x",
    issued: whileStopping.epoch,
  });
  assertEquals(failed.error, "");
  // The same answer from a read issued after the viewer was established
  // ends it: nothing runs there any more.
  const fresh = readTicket(generations, () => model);
  assertEquals(fresh.current(), true);
  const ended = reduceBrowser(model, { type: "read", read: late, now: 4, issued: fresh.epoch });
  assertEquals(ended.viewer, undefined);
});

test("a restart reopens only after that exact browser verifiably stopped", async () => {
  const signal = new AbortController().signal;
  const target = shown("ovh", 3);
  const none = { ...target.instance, instance_id: "", incarnation: 0 };
  assertEquals(stoppedFor(target, read(event("hibernated", { instance: target.instance }))), true);
  assertEquals(stoppedFor(target, read(event("hibernated", { instance: none }))), true);
  assertEquals(stoppedFor(target, read(event("stopping", { instance: target.instance }))), false);
  assertEquals(
    stoppedFor(target, read(event("hibernated", { instance: target.instance }), "beta")),
    false,
  );
  assertEquals(
    stoppedFor(target, read(event("hibernated", { instance: shown("ovh", 2).instance }))),
    false,
  );
  // The hibernate answered `stopping`: pinned reads until it hibernated.
  const answers = [
    read(event("stopping", { instance: target.instance })),
    read(event("hibernated", { instance: target.instance })),
  ];
  const reads: string[] = [];
  let waits = 0;
  await confirmStopped(
    target,
    { machine_id: "ovh", state: event("stopping", { instance: target.instance }) },
    () => {
      reads.push("read");
      return Promise.resolve(answers.shift() ?? read(null));
    },
    signal,
    () => {
      waits++;
      return Promise.resolve();
    },
  );
  assertEquals([reads.length, waits], [2, 2]);
  // A newer instance on that Machine, or no confirmation in time, refuses.
  const codeOf = async (answer: BrowserRead): Promise<string> => {
    try {
      await confirmStopped(
        target,
        answer,
        () => Promise.resolve(answer),
        signal,
        () => Promise.resolve(),
      );
      return "confirmed";
    } catch (cause) {
      return cause instanceof MachineBrowserError ? cause.code : "other";
    }
  };
  assertEquals(
    await codeOf(read(event("detached", { instance: shown("ovh", 4).instance }))),
    "browser_instance_changed",
  );
  let attempts = 0;
  const stuck = read(event("stopping", { instance: target.instance }));
  try {
    await confirmStopped(
      target,
      stuck,
      () => {
        attempts++;
        return Promise.resolve(stuck);
      },
      signal,
      () => Promise.resolve(),
    );
  } catch (cause) {
    assertEquals(cause instanceof MachineBrowserError && cause.code, "browser_restart_unconfirmed");
  }
  assertEquals(attempts, RESTART_CONFIRM_ATTEMPTS);
});
