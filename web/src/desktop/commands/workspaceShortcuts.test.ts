import { assertEquals } from "jsr:@std/assert";
import { chromeShortcutConflict } from "./chromeShortcutPolicy.ts";
import { macShortcutConflict } from "./macShortcutPolicy.ts";
import {
  DESKTOP_COMPOSER_FORMAT_CHORDS,
  DESKTOP_LEADER_GROUPS,
  DESKTOP_WORKSPACE_COMMANDS,
  DESKTOP_WORKSPACE_KEYS,
  DESKTOP_WORKSPACE_PREFIX,
  desktopLeaderKey,
  desktopLeaderLabel,
  isDesktopLeaderSpace,
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
    s: "group:s",
    p: "workspace.focusPrompt",
    t: "group:t",
    c: "workspace.focusConversation",
    l: "prompt.focusPlan",
    q: "prompt.focusQueue",
    d: "prompt.focusDrafts",
    n: "session.new",
    m: "composer.more",
    w: "group:w",
    u: "group:u",
    "/": "composer.slash",
    f: "composer.reference",
    a: "composer.attach",
    h: "composer.schedule",
    j: "composer.jumpFront",
    ",": "settings.open",
    r: "sync.retry",
    " ": "session.switch",
    "`": "session.alternate",
    k: "commandPalette.open",
    z: "editor.expand",
  });
});

Deno.test("pane collapse continuations use physical bracket keys", () => {
  assertEquals(
    desktopWorkspaceContinuationKey(
      keyEvent({ key: "ü", code: "BracketLeft", metaKey: true }),
      true,
    ),
    "[",
  );
  assertEquals(
    desktopWorkspaceContinuationKey(
      keyEvent({ key: "+", code: "BracketRight" }),
      true,
    ),
    "]",
  );
  assertEquals(
    desktopWorkspaceContinuationKey(
      keyEvent({ key: "#", code: "Backslash", altKey: true }),
      false,
    ),
    "\\",
  );
});

Deno.test("Space arms the leader only as a bare, non-repeated key", () => {
  const space = {
    code: "Space",
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    repeat: false,
  };
  assertEquals(isDesktopLeaderSpace(space), true);
  // macOS input-source switching, Shift+Space paging and held keys stay native.
  assertEquals(isDesktopLeaderSpace({ ...space, ctrlKey: true }), false);
  assertEquals(isDesktopLeaderSpace({ ...space, metaKey: true }), false);
  assertEquals(isDesktopLeaderSpace({ ...space, shiftKey: true }), false);
  assertEquals(isDesktopLeaderSpace({ ...space, repeat: true }), false);
  assertEquals(isDesktopLeaderSpace({ ...space, code: "KeyK" }), false);
});

Deno.test("leader keycaps draw the glyph and key in one label", () => {
  assertEquals(desktopLeaderLabel("n"), "␣N");
  assertEquals(desktopLeaderLabel(" "), "␣␣");
  assertEquals(desktopLeaderLabel("["), "␣[");
  assertEquals(
    desktopLeaderKey({ sequence: [DESKTOP_WORKSPACE_PREFIX, "N"] }),
    "n",
  );
  assertEquals(desktopLeaderKey({ sequence: ["G", "1"] }), null);
});

Deno.test("rich text uses direct chords, never the leader", () => {
  for (const [id, chord] of Object.entries(DESKTOP_COMPOSER_FORMAT_CHORDS)) {
    const command = `composer.format.${id}`;
    assertEquals(chord.startsWith("Mod+"), true);
    assertEquals(chromeShortcutConflict(command, chord, true), null);
    assertEquals(chromeShortcutConflict(command, chord, false), null);
    assertEquals(macShortcutConflict(command, chord), null);
  }
  assertEquals(DESKTOP_COMPOSER_FORMAT_CHORDS.bold, "Mod+B");
  assertEquals(DESKTOP_COMPOSER_FORMAT_CHORDS.italic, "Mod+I");
  assertEquals("m" in DESKTOP_LEADER_GROUPS, false);
});

Deno.test("surface groups double their key to focus and hold its buttons", () => {
  assertEquals(DESKTOP_LEADER_GROUPS.s, "Sessions");
  assertEquals(DESKTOP_WORKSPACE_KEYS.focusSessions, "SS");
  assertEquals(DESKTOP_WORKSPACE_KEYS.focusTopbar, "T");
  // The fold button beside Create (`␣SZ`, Vim's fold prefix).
  assertEquals(DESKTOP_WORKSPACE_KEYS.sessionsFold, "SZ");
  assertEquals(desktopLeaderLabel(DESKTOP_WORKSPACE_KEYS.sessionsFold), "␣SZ");
  for (
    const path of [
      DESKTOP_WORKSPACE_KEYS.sessionsNewFolder,
      DESKTOP_WORKSPACE_KEYS.sessionsMove,
      DESKTOP_WORKSPACE_KEYS.sessionsOrganize,
    ]
  ) {
    assertEquals(path.startsWith("S") && path.length === 2, true);
  }
});
