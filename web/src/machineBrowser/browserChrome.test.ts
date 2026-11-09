import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import type { BrowserRead, BrowserStateEvent } from "./api";
import { chromeState, parseAddress, retryOffered } from "./browserChrome";
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

test("a refused open for lack of memory offers Retry unless a Machine must be chosen", () => {
  const refused: BrowserModel = {
    ...shown("hibernated", { reason: "memory" }),
    error: "The Machine has no memory headroom for a browser right now.",
  };
  assertEquals(retryOffered(refused, false), true);
  assertEquals(retryOffered({ ...refused, error: "" }, false), false);
  assertEquals(retryOffered({ ...refused, busy: "open" }, false), false);
  const failed = {
    ...refused,
    read: { ...refused.read!, state: state("hibernated", { reason: "failed" }) },
  };
  assertEquals(retryOffered(failed, false), false);
  const choose: BrowserModel = {
    ...refused,
    read: { ...refused.read!, choose_machine: true, capable_machines: [{ machine_id: "ovh" }] },
  };
  assertEquals(retryOffered(choose, false), false);
  assertEquals(retryOffered(choose, true), true);
});
