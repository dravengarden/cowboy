import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import type { BrowserRead, BrowserStateEvent } from "./api";
import {
  CAP_WARNING_MS,
  attachedStopTime,
  browserStateSummary,
  capWarningDelay,
  capWarningText,
  chromeState,
  parseAddress,
  remainingText,
  retryOffered,
} from "./browserChrome";
import { type BrowserModel, INITIAL_BROWSER_MODEL, reduceBrowser } from "./browserModel";

const account = "11111111-2222-4333-8444-555555555555";

function state(
  value: BrowserStateEvent["state"],
  extra: Partial<BrowserStateEvent> = {},
): BrowserStateEvent {
  const live = value !== "hibernated";
  return {
    instance: {
      account_id: account,
      instance_id: live ? "b-aaaaaaaaaaaaaaaaaaaaaaaa" : "",
      incarnation: live ? 4 : 0,
    },
    state: value,
    profile_revision: 2,
    observed_at_ms: 1,
    ...extra,
  };
}

function shown(value: BrowserStateEvent["state"], extra: Partial<BrowserStateEvent> = {}) {
  const read: BrowserRead = {
    machine_id: "ovh",
    state: state(value, extra),
    placements: [],
    admission: null,
  };
  return reduceBrowser(INITIAL_BROWSER_MODEL, { type: "read", read, now: 1 });
}

test("addresses become https URLs and other schemes are refused, never rewritten", () => {
  assertEquals(parseAddress(" example.com "), { url: "https://example.com/" });
  assertEquals(parseAddress("example.com:8443/a?b=1"), { url: "https://example.com:8443/a?b=1" });
  assertEquals(parseAddress("https://claude.ai/login"), { url: "https://claude.ai/login" });
  assertEquals(parseAddress("HTTPS://Example.com"), { url: "https://example.com/" });
  for (const refused of [
    "http://example.com",
    "javascript:alert(1)",
    "file:///etc/passwd",
    "data:text/html,hi",
    "ftp://example.com",
  ]) {
    assertEquals("error" in parseAddress(refused), true);
  }
  assertEquals("error" in parseAddress(""), true);
  assertEquals("error" in parseAddress("two words"), true);
  assertEquals("error" in parseAddress("intranet"), true);
});

test("a hibernated browser offers only opening and the menu's applicable commands", () => {
  const chrome = chromeState(shown("hibernated"), false, 2);
  assertEquals(chrome.navigation, false);
  assertEquals(chrome.keyboard, false);
  assertEquals(chrome.address, false);
  assertEquals(chrome.hibernate, false);
  assertEquals(chrome.snapshot, false);
  assertEquals(chrome.detach, false);
  assertEquals(chrome.paste, false);
  assertEquals(chrome.copy, false);
  assertEquals(chrome.reset, true);
  assertEquals(chrome.refresh, true);
});

test("a running browser enables navigation only with a connected viewer", () => {
  const running = shown("detached", { scale: 2 });
  const idle = chromeState(running, false, 2);
  assertEquals(idle.address, true);
  assertEquals(idle.hibernate, true);
  assertEquals(idle.navigation, false);
  assertEquals(idle.reset, false);
  assertEquals(idle.restart, false);
  const viewing = chromeState(running, true, 2);
  assertEquals(viewing.navigation && viewing.keyboard && viewing.paste, true);
  // A different screen scale offers the restart.
  assertEquals(chromeState(running, true, 1).restart, true);
  // A command in flight disables every lifecycle command.
  const busy: BrowserModel = { ...running, busy: "snapshot" };
  const during = chromeState(busy, true, 2);
  assertEquals(during.hibernate || during.snapshot || during.address || during.refresh, false);
});

/** An open the Machine refused with `reason`, as the panel holds it. */
function refusedOpen(reason: string): BrowserModel {
  const opening = reduceBrowser(shown("hibernated"), { type: "busy", action: "open" });
  return reduceBrowser(opening, {
    type: "result",
    action: "open",
    result: { machine_id: "ovh", state: state("hibernated", { reason }) },
    now: 2,
  });
}

test("a refused open for lack of memory offers Retry unless a Machine must be chosen", () => {
  const refused = refusedOpen("memory");
  assertEquals(refused.refusal, "memory");
  assertEquals(retryOffered(refused, false), true);
  assertEquals(retryOffered({ ...refused, error: "" }, false), false);
  assertEquals(retryOffered({ ...refused, busy: "open" }, false), false);
  // A failure that does not free up by itself offers none.
  assertEquals(retryOffered(refusedOpen("failed"), false), false);
  const choose: BrowserModel = {
    ...refused,
    read: { ...refused.read!, choose_machine: true, capable_machines: [{ machine_id: "ovh" }] },
  };
  assertEquals(retryOffered(choose, false), false);
  assertEquals(retryOffered(choose, true), true);
});

test("Retry stays with the refusal while state reads report the last stop", () => {
  const refused = refusedOpen("capacity");
  assertEquals(retryOffered(refused, false), true);
  // The next poll says why the browser last stopped, not why the open was
  // refused: the banner and its Retry stay.
  const polled = reduceBrowser(refused, {
    type: "read",
    read: {
      machine_id: "ovh",
      state: state("hibernated", { reason: "grace" }),
      placements: [],
      admission: null,
    },
    now: 3,
    issued: refused.epoch,
  });
  assertEquals(polled.read?.state?.reason, "grace");
  assertEquals(polled.refusal, "capacity");
  assertEquals(polled.error.length > 0, true);
  assertEquals(retryOffered(polled, false), true);
  // Dismissing the banner, or retrying, lets the refusal go with it.
  assertEquals(reduceBrowser(polled, { type: "dismiss" }).refusal, undefined);
  assertEquals(retryOffered(reduceBrowser(polled, { type: "dismiss" }), false), false);
  assertEquals(reduceBrowser(polled, { type: "busy", action: "open" }).refusal, undefined);
});

test("a detached browser's summary counts down to the end of its grace", () => {
  const now = 1_000_000;
  // The Machine reports when it stops if nothing changes: the grace's end.
  assertEquals(
    browserStateSummary(state("detached", { deadline_ms: now + 95_400 }), now),
    "Running, not attached · Hibernates in about 2 min unless attached",
  );
  assertEquals(
    browserStateSummary(state("detached", { deadline_ms: now + 42_000 }), now),
    "Running, not attached · Hibernates in 42 s unless attached",
  );
  // Already due, or a clock ahead of the Machine's: never negative.
  assertEquals(
    browserStateSummary(state("detached", { deadline_ms: now - 5_000 }), now),
    "Running, not attached · Hibernates in 0 s unless attached",
  );
  // An attached browser's stop time is its runtime cap, not a grace.
  assertEquals(
    browserStateSummary(state("attached", { deadline_ms: now + 42_000 }), now),
    "Attached",
  );
  assertEquals(
    browserStateSummary(state("hibernated", { reason: "reclaim", unsaved_state_lost: true }), now),
    "Hibernated · The Machine needed this browser’s resources for other work and hibernated it. · The last stop lost unsaved page state",
  );
  assertEquals(remainingText(89_400), "89 s");
  assertEquals(remainingText(90_000), "about 2 min");
  assertEquals(remainingText(28 * 60_000), "about 28 min");
});

test("an attached viewer is warned two minutes ahead of the runtime cap", () => {
  const now = 5_000_000;
  const stopsAt = now + 30 * 60_000;
  const mine = state("attached", { deadline_ms: stopsAt, attachment_id: "att-me" });
  assertEquals(attachedStopTime(mine, "att-me"), stopsAt);
  // Not before the stream confirmed this device's attachment, and never for
  // the display another device still holds while a takeover is requested.
  assertEquals(attachedStopTime(mine, undefined), undefined);
  assertEquals(attachedStopTime(mine, "att-other"), undefined);
  assertEquals(attachedStopTime(state("attached", { deadline_ms: stopsAt }), "att-me"), undefined);
  // Nothing to warn about while nobody views it, or without a stop time.
  assertEquals(
    attachedStopTime(
      state("detached", { deadline_ms: stopsAt, attachment_id: "att-me" }),
      "att-me",
    ),
    undefined,
  );
  assertEquals(
    attachedStopTime(state("attached", { attachment_id: "att-me" }), "att-me"),
    undefined,
  );
  assertEquals(attachedStopTime(null, "att-me"), undefined);
  assertEquals(capWarningDelay(stopsAt, now), 30 * 60_000 - CAP_WARNING_MS);
  // Attached (or reattached) inside the last two minutes: told at once.
  assertEquals(capWarningDelay(stopsAt, stopsAt - 45_000), 0);
  assertEquals(capWarningDelay(stopsAt, stopsAt + 1), 0);
  assertEquals(
    capWarningText(stopsAt, stopsAt - CAP_WARNING_MS),
    "This browser reaches its running-time limit in about 2 min and is then hibernated. Finish the step you are on: opening it again starts on a new tab, with your sign-ins as last saved.",
  );
  assertEquals(capWarningText(stopsAt, stopsAt - 45_000).includes("in 45 s"), true);
});
