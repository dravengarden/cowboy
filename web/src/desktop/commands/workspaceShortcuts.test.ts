import { assertEquals } from "jsr:@std/assert";
import {
  DESKTOP_WORKSPACE_COMMANDS,
  desktopWorkspaceContinuationKey,
  matchesDesktopWorkspacePrefix,
} from "./workspaceShortcuts.ts";

function keyEvent(overrides: Partial<KeyboardEvent>): KeyboardEvent {
  return {
    key: "k",
    code: "KeyK",
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    ...overrides,
  } as KeyboardEvent;
}

Deno.test("workspace prefix matches Command-K on macOS and Alt-K elsewhere", () => {
  assertEquals(
    matchesDesktopWorkspacePrefix(keyEvent({ metaKey: true }), true),
    true,
  );
  assertEquals(
    matchesDesktopWorkspacePrefix(keyEvent({ ctrlKey: true }), true),
    false,
  );
  assertEquals(
    matchesDesktopWorkspacePrefix(keyEvent({ altKey: true }), false),
    true,
  );
  assertEquals(
    matchesDesktopWorkspacePrefix(keyEvent({ ctrlKey: true }), false),
    false,
  );
});

Deno.test("continuations use physical keys with or without the held prefix modifier", () => {
  const physicalS = { key: "ß", code: "KeyS" };
  assertEquals(desktopWorkspaceContinuationKey(keyEvent(physicalS), true), "s");
  assertEquals(
    desktopWorkspaceContinuationKey(
      keyEvent({ ...physicalS, metaKey: true }),
      true,
    ),
    "s",
  );
  assertEquals(
    desktopWorkspaceContinuationKey(
      keyEvent({ ...physicalS, altKey: true }),
      false,
    ),
    "s",
  );
  assertEquals(
    desktopWorkspaceContinuationKey(
      keyEvent({ ...physicalS, ctrlKey: true }),
      false,
    ),
    null,
  );
});

Deno.test("every prefix continuation has one stable command meaning", () => {
  assertEquals(DESKTOP_WORKSPACE_COMMANDS, {
    s: "workspace.focusSessions",
    p: "workspace.focusPrompt",
    t: "workspace.focusTopbar",
    c: "workspace.focusConversation",
    l: "prompt.focusPlan",
    q: "prompt.focusQueue",
    d: "prompt.focusDrafts",
    n: "session.new",
    w: "workspace.cycleRegion",
    r: "workspace.enterResize",
    ",": "settings.open",
  });
});
