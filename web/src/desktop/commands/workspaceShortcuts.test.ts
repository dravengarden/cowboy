import { assertEquals } from "jsr:@std/assert";
import {
  DESKTOP_WORKSPACE_COMMANDS,
  desktopWorkspaceContinuationKey,
  desktopWorkspaceSequenceOwnsKey,
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

Deno.test("workspace sequence preempts idle IME markers but not real composition", () => {
  const idleImePrefix = keyEvent({
    key: "Process",
    code: "KeyK",
    keyCode: 229,
    metaKey: true,
  });
  assertEquals(
    desktopWorkspaceSequenceOwnsKey(idleImePrefix, false, false, true),
    true,
  );
  assertEquals(
    desktopWorkspaceSequenceOwnsKey(
      keyEvent({ metaKey: true, isComposing: true }),
      false,
      false,
      true,
    ),
    false,
  );
  assertEquals(
    desktopWorkspaceSequenceOwnsKey(idleImePrefix, false, true, true),
    false,
  );

  const idleImeContinuation = keyEvent({
    key: "Process",
    code: "KeyP",
    keyCode: 229,
  });
  assertEquals(
    desktopWorkspaceSequenceOwnsKey(idleImeContinuation, true, false, true),
    true,
  );
  assertEquals(
    desktopWorkspaceSequenceOwnsKey(idleImeContinuation, true, true, true),
    false,
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
