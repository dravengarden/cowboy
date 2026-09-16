import { assert, assertEquals } from "jsr:@std/assert";
import {
  getComposerSourceMode,
  setComposerSourceMode,
  toggleComposerSourceMode,
} from "./composerSourceMode.ts";
import {
  DEFAULT_COMPOSER_TOOLBAR,
  normalizeComposerToolbarOrder,
} from "./composerToolbarModel.ts";
import {
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_COMMANDS,
  DESKTOP_WORKSPACE_KEYS,
  DESKTOP_WORKSPACE_PREFIX,
} from "./desktop/commands/workspaceShortcuts.ts";
import { chromeShortcutConflict } from "./desktop/commands/chromeShortcutPolicy.ts";
import { macShortcutConflict } from "./desktop/commands/macShortcutPolicy.ts";

const extensions = await Deno.readTextFile(
  new URL("./composerExtensions.ts", import.meta.url),
);
const editor = await Deno.readTextFile(
  new URL("./ComposerEditor.tsx", import.meta.url),
);
const commands = await Deno.readTextFile(
  new URL("./composerCommands.tsx", import.meta.url),
);
const composerBindings = await Deno.readTextFile(
  new URL("./desktop/commands/DesktopComposerShortcuts.tsx", import.meta.url),
);
const statusLine = await Deno.readTextFile(
  new URL("./desktop/DesktopStatusLine.tsx", import.meta.url),
);
const shortcutsDialog = await Deno.readTextFile(
  new URL("./desktop/commands/DesktopShortcutsDialog.tsx", import.meta.url),
);
const app = await Deno.readTextFile(new URL("./App.tsx", import.meta.url));
const preview = await Deno.readTextFile(
  new URL("./MessagePreview.tsx", import.meta.url),
);

Deno.test("source mode is one persisted preference the whole app toggles", () => {
  setComposerSourceMode(false);
  assertEquals(getComposerSourceMode(), false);
  assertEquals(toggleComposerSourceMode(), true);
  assertEquals(getComposerSourceMode(), true);
  assertEquals(toggleComposerSourceMode(), false);
  setComposerSourceMode(false);
});

// Source mode drops the DECORATION engine and nothing else: the document, the
// editing behaviour and cowboy's own token widgets stay identical, so a toggle
// can never rewrite what the user typed.
Deno.test("source mode omits the live-preview engine, keeping every other extension", () => {
  const body = extensions.slice(
    extensions.indexOf("export function livePreviewExtensions"),
  );
  const branch = body.indexOf("...(opts.sourceMode");
  assert(branch > 0);
  const sourceBranch = body.slice(
    branch,
    body.indexOf("EditorView.lineWrapping", branch),
  );
  const [rendered, raw] = [
    sourceBranch.indexOf("inlinePreview({"),
    sourceBranch.indexOf('"data-composer-source": "true"'),
  ];
  assert(
    raw > 0 && rendered > raw,
    "the raw branch must precede the rendered one",
  );
  // Everything below lives OUTSIDE the branch, i.e. in both modes.
  for (
    const shared of [
      "markdown({ base: markdownLanguage",
      "atomicMarkdownSyntax",
      "atomicEditorTheme",
      "closeBrackets()",
      "extendEmphasisPair",
      "autoCloseCodeFence",
      "highlightActiveLine()",
      "EditorView.lineWrapping",
    ]
  ) {
    assert(body.includes(shared), `${shared} must stay in both modes`);
    assertEquals(sourceBranch.includes(shared), false);
  }
});

Deno.test("every composer surface reads the same mode, and the preview never does", () => {
  assert(editor.includes("const sourceMode = useComposerSourceMode();"));
  assert(editor.includes("...livePreviewExtensions({ sourceMode }),"));
  // In the extension memo's dependency list, so a flip reconfigures CM6.
  const deps = editor.slice(
    editor.indexOf("...livePreviewExtensions({ sourceMode }),"),
  );
  assert(deps.slice(0, 400).includes("sourceMode,"));
  // The queued/draft read-only preview is a RENDERED view of a message, not a
  // writing surface — Obsidian's reading view ignores source mode too.
  assert(preview.includes("...livePreviewExtensions(),"));
  assertEquals(preview.includes("sourceMode"), false);
});

Deno.test("mobile reaches source mode through the curatable toolbar registry", () => {
  assert(commands.includes('id: "sourceMode"'));
  assert(commands.includes("toggleComposerSourceMode()"));
  assert(DEFAULT_COMPOSER_TOOLBAR.includes("sourceMode"));
  // An uncurated device that still holds the retired default is carried
  // forward; a curated order stays exactly as the user left it.
  const retired = DEFAULT_COMPOSER_TOOLBAR.filter((id) => id !== "sourceMode");
  assertEquals(
    normalizeComposerToolbarOrder(retired, () => true),
    [...DEFAULT_COMPOSER_TOOLBAR],
  );
  assertEquals(
    normalizeComposerToolbarOrder(["bold", "italic"], () => true),
    ["bold", "italic"],
  );
});

// Obsidian's Mod+E is unavailable to Cowboy, so the toggle uses FOCUS.md's
// documented fallback: the platform workspace prefix. Keep this test as the
// executable record of WHY the binding is a sequence.
Deno.test("source mode uses the workspace prefix because Mod+E is reserved", () => {
  assert(chromeShortcutConflict("composer.toggleSourceMode", "Mod+E", true));
  assert(macShortcutConflict("composer.toggleSourceMode", "Mod+E"));
  assert(chromeShortcutConflict("composer.toggleSourceMode", "Alt+E", false));
  assertEquals(DESKTOP_WORKSPACE_KEYS.toggleSourceMode, "E");
  assertEquals(
    DESKTOP_SHORTCUTS.toggleSourceMode,
    `${DESKTOP_WORKSPACE_PREFIX} → E`,
  );
  assertEquals(DESKTOP_WORKSPACE_COMMANDS["e"], "composer.toggleSourceMode");
  // A sequence, never a direct chord: a direct binding would have to pass the
  // browser audits that just rejected Mod+E.
  const binding = composerBindings.slice(
    composerBindings.indexOf('id: "composer.toggleSourceMode"'),
  );
  const command = binding.slice(0, binding.indexOf("},"));
  assert(command.includes("DESKTOP_WORKSPACE_KEYS.toggleSourceMode"));
  assert(command.includes("allowInEditor: true"));
  assert(command.includes('contexts: ["prompt"]'));
  assertEquals(command.includes("shortcut:"), false);
  // Not region-pinned: the queue and draft editors are composers too.
  assertEquals(command.includes("regions:"), false);
});

Deno.test("the mode is discoverable without the palette", () => {
  // Status line, while the composer owns focus and in workspace command mode.
  assert(statusLine.includes("keys: DESKTOP_SHORTCUTS.toggleSourceMode"));
  assert(statusLine.includes("keys: DESKTOP_WORKSPACE_KEYS.toggleSourceMode"));
  assert(statusLine.includes('sourceMode ? "Live preview" : "Source"'));
  // The shortcut guide and the Settings toggle.
  assert(shortcutsDialog.includes("DESKTOP_SHORTCUTS.toggleSourceMode"));
  assert(app.includes('label="Source mode"'));
  assert(app.includes("setComposerSourceMode(!sourceMode)"));
});
