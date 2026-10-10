import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const provider = await readFile(
  new URL("./DesktopCommandProvider.tsx", import.meta.url), "utf8",
);
const host = await readFile(
  new URL("./DesktopCommandHost.tsx", import.meta.url), "utf8",
);
const composerBindings = await readFile(
  new URL("./DesktopComposerShortcuts.tsx", import.meta.url), "utf8",
);
const pendingBindings = await readFile(
  new URL("./DesktopPendingEditShortcuts.tsx", import.meta.url), "utf8",
);
const composer = await readFile(
  new URL("../../Composer.tsx", import.meta.url), "utf8",
);
const topbar = await readFile(
  new URL("../DesktopTopBarControls.tsx", import.meta.url), "utf8",
);
const workspaceController = await readFile(
  new URL("../DesktopWorkspaceController.tsx", import.meta.url), "utf8",
);
const vimRuntime = await readFile(
  new URL("../vim/imeAutoInsertVim.ts", import.meta.url), "utf8",
);

test("workspace prefix has priority after IME and exclusive overlays", () => {
  const arbitration = provider.indexOf("desktopWorkspaceSequenceOwnsKey(");
  const ime = provider.indexOf("desktopImeOwnsKey(event)");
  const overlay = provider.indexOf("if (desktopOverlayOwnsShortcuts(document)) {");
  // The modal grammar runs before the strict IME gate, but only for keys
  // desktopKeyIntent does not give to a composition.
  const modal = provider.indexOf("handleDesktopModalKey(event, modal)");
  assert(modal >= 0 && modal < ime);
  assert(
    provider.slice(modal - 400, modal).includes(
      'desktopKeyIntent(event).owner !== "ime"',
    ),
  );
  const prefix = provider.indexOf("matchesDesktopWorkspacePrefix(event)");
  const direct = provider.indexOf(
    "for (const command of commands.current.values())",
  );
  assert(arbitration >= 0 && arbitration < ime);
  assert(ime < overlay);
  assert(overlay < prefix);
  assert(prefix < direct);
});

test("claimed workspace strokes stop same-node Vim listeners immediately", () => {
  const prefix = provider.indexOf("if (matchesDesktopWorkspacePrefix(event))");
  const continuation = provider.indexOf(
    "if (leaderArmed.current) {",
    prefix,
  );
  assert(prefix >= 0);
  assert(continuation > prefix);
  assert(
    provider.slice(prefix, prefix + 600).includes(
      "event.stopImmediatePropagation()",
    ),
  );
  assert(
    provider.slice(continuation, continuation + 1600).includes(
      "event.stopImmediatePropagation()",
    ),
  );
});

test("workspace destinations are sequences rather than direct bare keys", () => {
  for (
    const key of [
      "focusSessions",
      "focusPrompt",
      "focusTopbar",
      "focusConversation",
      "focusPlan",
      "focusQueue",
      "focusDrafts",
      "newSession",
      "cycleRegion",
      "resize",
      "settings",
    ]
  ) {
    assert(host.includes(`DESKTOP_WORKSPACE_KEYS.${key}`));
  }
  assertEquals(host.includes('id: "conversation.focusTranscript"'), false);
});

test("low-frequency Option letter bindings are palette-only", () => {
  for (
    const shortcut of [
      "Alt+/",
      "Alt+R",
      "Alt+A",
      "Alt+S",
      "Alt+J",
      "Alt+X",
      "Alt+E",
    ]
  ) {
    assertEquals(composerBindings.includes(`shortcut: "${shortcut}"`), false);
    assertEquals(pendingBindings.includes(`shortcut: "${shortcut}"`), false);
  }
  for (const hint of ["/", "R", "A", "S", "J", "X", "E"]) {
    assertEquals(composer.includes("`${ALT_LABEL}" + hint + "`,"), false);
  }
  assert(composerBindings.includes('shortcut: "Alt+Enter"'));
  assert(composerBindings.includes("DESKTOP_SHORTCUTS.saveDraft"));
});

test("Stop is global Mod-period and Escape no longer arms navigation", () => {
  const start = topbar.indexOf('id: "topbar.stop"');
  const stop = topbar.slice(start, topbar.indexOf("], []);", start));
  assert(stop.includes("shortcut: DESKTOP_SHORTCUTS.stop"));
  assert(stop.includes("allowInEditor: true"));
  assertEquals(stop.includes("regions:"), false);
  assertEquals(composer.includes("DESKTOP_WORKSPACE_COMMAND_EVENT"), false);
  assertEquals(composer.includes("cowboy:desktop-workspace-command"), false);
});

test("returning to Prompt preserves Vim mode and caret", () => {
  assert(workspaceController.includes("composer ?? composerCommandSink"));
  assert(vimRuntime.includes('closest("[data-desktop-region]") !== null'));
  assert(vimRuntime.includes("Workspace-prefix navigation is focus movement"));
});
