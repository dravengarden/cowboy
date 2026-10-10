import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  OPEN_SESSION_SETTINGS_EVENT,
  openSessionSettings,
  sessionSettingsFocusFromEvent,
} from "./sessionSettingsOpen.ts";

const transcriptSource = await readFile(
  new URL("Transcript.tsx", import.meta.url), "utf8",
);
const composerSource = await readFile(
  new URL("Composer.tsx", import.meta.url), "utf8",
);

test("empty-state chips open the session settings switcher", () => {
  assertEquals(
    transcriptSource.includes('data-conversation-empty-settings'),
    true,
  );
  assertEquals(
    transcriptSource.includes('openSessionSettings("agent")'),
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
