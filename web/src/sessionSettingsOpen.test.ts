import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  OPEN_SESSION_SETTINGS_EVENT,
  pendingSessionSettings,
  openSessionSettings,
  sessionSettingsFocusFromEvent,
  sessionSettingsEventMatches,
} from "./sessionSettingsOpen.ts";
import { readComposerSource, readTranscriptSource } from "./appSource.test.ts";

const transcriptSource = await readTranscriptSource();
const composerSource = await readComposerSource();

test("empty-state chips open the session settings switcher", () => {
  assertEquals(
    transcriptSource.includes('data-conversation-empty-settings'),
    true,
  );
  assertEquals(
    transcriptSource.includes('openSessionSettings("agent", sessionId)'),
    true,
  );
  assertEquals(
    composerSource.includes("OPEN_SESSION_SETTINGS_EVENT"),
    true,
  );
  assertEquals(
    composerSource.includes("data-session-settings-agent"),
    true,
  );
});

test("session settings open events keep an explicit focus", () => {
  assertEquals(
    sessionSettingsFocusFromEvent(
      new CustomEvent(OPEN_SESSION_SETTINGS_EVENT, { detail: { focus: "agent" } }),
    ),
    "agent",
  );
  assertEquals(
    sessionSettingsFocusFromEvent(
      new CustomEvent(OPEN_SESSION_SETTINGS_EVENT),
    ),
    "session",
  );
  assertEquals(typeof openSessionSettings, "function");
});

test("a settings action targets its session in a split workspace", () => {
  const event = new CustomEvent(OPEN_SESSION_SETTINGS_EVENT, { detail: { focus: "agent", sessionId: "one" } });
  assertEquals(sessionSettingsEventMatches(event, "one"), true);
  assertEquals(sessionSettingsEventMatches(event, "two"), false);
});

test("a request a mounted surface took does not replay on remount", () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, "document");
  const events = new EventTarget();
  Object.defineProperty(globalThis, "document", { value: events, configurable: true });
  const take = (event: Event): void => {
    sessionSettingsEventMatches(event, "one");
  };
  events.addEventListener(OPEN_SESSION_SETTINGS_EVENT, take);
  try {
    openSessionSettings("agent", "one");
    assertEquals(pendingSessionSettings("one"), null);
    // No mounted surface for "two": its mount still honors the request.
    openSessionSettings("agent", "two");
    assertEquals(pendingSessionSettings("two"), "agent");
  } finally {
    events.removeEventListener(OPEN_SESSION_SETTINGS_EVENT, take);
    if (original) Object.defineProperty(globalThis, "document", original);
    else delete (globalThis as { document?: unknown }).document;
  }
});
