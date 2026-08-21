import { assert, assertEquals } from "jsr:@std/assert";

const provider = await Deno.readTextFile(
  new URL("./DesktopCommandProvider.tsx", import.meta.url),
);
const host = await Deno.readTextFile(
  new URL("./DesktopCommandHost.tsx", import.meta.url),
);
const composerBindings = await Deno.readTextFile(
  new URL("./DesktopComposerShortcuts.tsx", import.meta.url),
);
const pendingBindings = await Deno.readTextFile(
  new URL("./DesktopPendingEditShortcuts.tsx", import.meta.url),
);
const composer = await Deno.readTextFile(
  new URL("../../Composer.tsx", import.meta.url),
);
const topbar = await Deno.readTextFile(
  new URL("../DesktopTopBarControls.tsx", import.meta.url),
);
const workspaceController = await Deno.readTextFile(
  new URL("../DesktopWorkspaceController.tsx", import.meta.url),
);
const vimRuntime = await Deno.readTextFile(
  new URL("../vim/imeAutoInsertVim.ts", import.meta.url),
);

Deno.test("workspace prefix has priority after IME and exclusive overlays", () => {
  const arbitration = provider.indexOf("desktopWorkspaceSequenceOwnsKey(");
  const ime = provider.indexOf("desktopImeOwnsKey(event)");
  const overlay = provider.indexOf("desktopOverlayOwnsShortcuts(document)");
  const prefix = provider.indexOf("matchesDesktopWorkspacePrefix(event)");
  const reading = provider.indexOf('workspace.productMode === "reading"');
  const direct = provider.indexOf(
    "for (const command of commands.current.values())",
  );
  assert(arbitration >= 0 && arbitration < ime);
  assert(ime < overlay);
  assert(overlay < prefix);
  assert(prefix < reading);
  assert(reading < direct);
});

Deno.test("claimed workspace strokes stop same-node Vim listeners immediately", () => {
  const prefix = provider.indexOf("if (matchesDesktopWorkspacePrefix(event))");
  const continuation = provider.indexOf(
    "if (workspaceCommandTimer.current !== null)",
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

Deno.test("workspace destinations are sequences rather than direct bare keys", () => {
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

Deno.test("low-frequency Option letter bindings are palette-only", () => {
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

Deno.test("Stop is global Mod-period and Escape no longer arms navigation", () => {
  const start = topbar.indexOf('id: "topbar.stop"');
  const stop = topbar.slice(start, topbar.indexOf("], []);", start));
  assert(stop.includes("shortcut: DESKTOP_SHORTCUTS.stop"));
  assert(stop.includes("allowInEditor: true"));
  assertEquals(stop.includes("regions:"), false);
  assertEquals(composer.includes("DESKTOP_WORKSPACE_COMMAND_EVENT"), false);
  assertEquals(composer.includes("cowboy:desktop-workspace-command"), false);
});

Deno.test("returning to Prompt preserves Vim mode and caret", () => {
  assert(workspaceController.includes("composer ?? composerCommandSink"));
  assert(vimRuntime.includes('closest("[data-desktop-region]") !== null'));
  assert(vimRuntime.includes("Workspace-prefix navigation is focus movement"));
});
