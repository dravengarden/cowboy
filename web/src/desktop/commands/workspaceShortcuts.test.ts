import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { chromeShortcutConflict } from "./chromeShortcutPolicy.ts";
import { macShortcutConflict } from "./macShortcutPolicy.ts";
import {
  DESKTOP_COMPOSER_FORMAT_CHORDS,
  DESKTOP_LEADER_GROUPS,
  DESKTOP_WORKSPACE_COMMANDS,
  DESKTOP_WORKSPACE_KEYS,
  DESKTOP_DOCUMENT_COMMANDS,
  DESKTOP_DOCUMENT_KEYS,
  desktopLeaderSequence,
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

test("workspace prefix matches Command-K on macOS and Alt-K elsewhere", () => {
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

test("continuations use physical keys with or without the held prefix modifier", () => {
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

test("workspace sequence preempts idle IME markers but not real composition", () => {
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

test("every prefix continuation has one stable command meaning", () => {
  assertEquals(DESKTOP_WORKSPACE_COMMANDS, {
    s: "group:s",
    p: "workspace.focusPrompt",
    i: "workspace.insertPrompt",
    t: "group:t",
    c: "workspace.focusConversation",
    v: "review.open",
    l: "prompt.focusPlan",
    g: "prompt.focusCalls",
    q: "prompt.focusQueue",
    d: "prompt.focusDrafts",
    n: "session.new",
    m: "composer.more",
    w: "group:w",
    z: "group:z",
    u: "group:u",
    b: "group:b",
    "[": "tabs.previous",
    "]": "tabs.next",
    "/": "composer.slash",
    f: "composer.reference",
    a: "composer.attach",
    h: "composer.schedule",
    j: "composer.jumpFront",
    ",": "settings.open",
    x: "workspace.extensions",
    ".": "sync.retry",
    r: "item.rename",
    o: "session.recent",
    " ": "session.switch",
    tab: "session.alternate",
    k: "commandPalette.open",
  });
});

test("Fold group uses mnemonic pane letters, Zoom and Expand All", () => {
  assertEquals(DESKTOP_LEADER_GROUPS.z, "Fold");
  assertEquals(DESKTOP_WORKSPACE_KEYS.toggleSessions, "ZS");
  assertEquals(DESKTOP_WORKSPACE_KEYS.togglePrompt, "ZP");
  assertEquals(DESKTOP_WORKSPACE_KEYS.toggleConversation, "ZC");
  assertEquals(DESKTOP_WORKSPACE_KEYS.toggleZoom, "ZZ");
  assertEquals(DESKTOP_WORKSPACE_KEYS.expandAllPanes, "ZA");
  assertEquals(desktopLeaderLabel(DESKTOP_WORKSPACE_KEYS.toggleSessions), "␣ZS");
  assertEquals(desktopLeaderLabel(DESKTOP_WORKSPACE_KEYS.togglePrompt), "␣ZP");
  assertEquals(desktopLeaderLabel(DESKTOP_WORKSPACE_KEYS.toggleConversation), "␣ZC");
  assertEquals(desktopLeaderLabel(DESKTOP_WORKSPACE_KEYS.toggleZoom), "␣ZZ");
  assertEquals(desktopLeaderLabel(DESKTOP_WORKSPACE_KEYS.expandAllPanes), "␣ZA");
  // Physical bracket keys remain positional for tab stepping (`␣[` / `␣]`).
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
});

test("Space arms the leader only as a bare, non-repeated key", () => {
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

test("leader keycaps draw the glyph and key in one label", () => {
  assertEquals(desktopLeaderLabel("n"), "␣N");
  assertEquals(desktopLeaderLabel(" "), "␣␣");
  assertEquals(desktopLeaderLabel("["), "␣[");
  assertEquals(
    desktopLeaderKey({ sequence: [DESKTOP_WORKSPACE_PREFIX, "N"] }),
    "n",
  );
  assertEquals(desktopLeaderKey({ sequence: ["G", "1"] }), null);
});

test("rich text uses direct chords, never the leader", () => {
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

test("surface groups double their key to focus and hold its buttons", () => {
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

test("previous item is Space-Tab, drawn as one keycap", () => {
  assertEquals(desktopLeaderLabel(DESKTOP_WORKSPACE_KEYS.alternateSession), "␣⇥");
  assertEquals(desktopLeaderLabel("Tab"), "␣⇥");
  assertEquals(
    desktopLeaderKey({
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.alternateSession),
    }),
    "tab",
  );
  assertEquals(DESKTOP_WORKSPACE_KEYS.recentSessions, "O");
  assertEquals(DESKTOP_WORKSPACE_KEYS.rename, "R");
});

test("an open Draft's actions are root keys, not a group", () => {
  assertEquals("d" in DESKTOP_LEADER_GROUPS, false);
  assertEquals(DESKTOP_DOCUMENT_KEYS.copy, "Y");
  assertEquals(DESKTOP_DOCUMENT_KEYS.readableWidth, "UW");
  for (const key of Object.keys(DESKTOP_DOCUMENT_COMMANDS)) {
    // Only surface-scoped meanings: Schedule (`H`) and Rename (`R`),
    // which maps to the Draft's title on a Draft page.
    const session = DESKTOP_WORKSPACE_COMMANDS[key];
    assertEquals(
      session === undefined || session === "composer.schedule" ||
        session === "item.rename",
      true,
    );
  }
});
