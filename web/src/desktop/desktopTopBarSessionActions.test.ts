import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import { readComposerSource } from "../appSource.test.ts";

const topBarSource = await readFile(
  new URL("./DesktopTopBarControls.tsx", import.meta.url), "utf8",
);
const composerSource = await readComposerSource();
const toolbarSource = await readFile(
  new URL("./DesktopComposerToolbar.tsx", import.meta.url), "utf8",
);
const sessionActionsSource = await readFile(
  new URL("./useDesktopSessionActions.tsx", import.meta.url), "utf8",
);
const embeddedControlSource = await readFile(
  new URL("./DesktopEmbeddedControl.ts", import.meta.url), "utf8",
);
const shortcutDialogSource = await readFile(
  new URL("./commands/DesktopShortcutsDialog.tsx", import.meta.url), "utf8",
);
const focusContract = await readFile(
  new URL("./FOCUS.md", import.meta.url), "utf8",
);

test("desktop session verification lives in Top Bar with a ␣TA slot", () => {
  const auth = topBarSource.indexOf("data-product-session-alert-control");
  const reload = topBarSource.indexOf('data-desktop-item="topbar-reload"');

  assert(auth >= 0);
  assert(reload > auth);
  assert(topBarSource.includes("data-product-session-alert-host"));
  assert(topBarSource.includes('id: "topbar.reauthenticate"'));
  assert(topBarSource.includes('shortcut: "A"'));
  assert(topBarSource.includes('sequence: [DESKTOP_WORKSPACE_PREFIX, "T", "A"]'));
  assert(topBarSource.includes('<LeaderKeycap leaderKey="TA" />'));
  assert(topBarSource.includes("desktopSessionActionSx({ minWidth: 112 })"));
  // The guide shows the Top bar group slot (`␣TA`, from any focus) beside
  // the bare letter that works while the bar owns focus.
  assert(
    shortcutDialogSource.includes(
      '["A", "Verify Product Session (while it is requested)"]',
    ),
  );
  assert(
    shortcutDialogSource.includes(
      "desktopLeaderGroupSequence(DESKTOP_WORKSPACE_KEYS.focusTopbar, key), key",
    ),
  );
  assert(focusContract.includes("top-bar `R`/`U`/`A`/`L`/`C`/`X`"));
});

test("Compact, Clear and Stop leave the Top bar for the Prompt", () => {
  for (const item of ["topbar-compact", "topbar-clear", "topbar-stop"]) {
    assertEquals(topBarSource.includes(item), false);
  }
  assertEquals(topBarSource.includes("<AutoScrollAndStop"), false);
  assertEquals(topBarSource.includes('id: "topbar.compact"'), false);
  assertEquals(topBarSource.includes('id: "topbar.clear"'), false);
  assertEquals(topBarSource.includes('id: "topbar.stop"'), false);
  assert(topBarSource.includes('data-desktop-item="topbar-reload"'));
  assert(topBarSource.includes("<SessionReloadDialog"));
  assert(topBarSource.includes('keyLabel="L"'));
  assert(topBarSource.includes('shortcut: "L"'));
  // The delivery row: conversation actions after Schedule, Stop before Send.
  const schedule = toolbarSource.indexOf('id: "schedule"');
  const compact = toolbarSource.indexOf('id: "compact"');
  const clear = toolbarSource.indexOf('id: "clear"');
  const stop = toolbarSource.indexOf('id: "stop"');
  const send = toolbarSource.indexOf('data-composer-action="send"');
  assert(schedule >= 0 && compact > schedule && clear > compact);
  assert(stop > clear && send > stop);
  // Compact and Clear are Prompt-scoped chords, not a leader group.
  assert(toolbarSource.includes("shortcut: DESKTOP_SHORTCUTS.compact"));
  assert(toolbarSource.includes("shortcut: DESKTOP_SHORTCUTS.clear"));
  assert(toolbarSource.includes("shortcut: DESKTOP_SHORTCUTS.stop"));
  assert(sessionActionsSource.includes("await resetSession(sessionId)"));
  assert(sessionActionsSource.includes('id: "prompt.compact"'));
  assert(sessionActionsSource.includes('id: "prompt.clear"'));
  assert(!sessionActionsSource.includes("leaderAnywhere: true"));
  assert(sessionActionsSource.includes('regions: ["prompt.composer"]'));
  assert(sessionActionsSource.includes("...PROMPT_SCOPE"));
});

test("desktop session actions share compact geometry without duplicate context text", () => {
  const start = topBarSource.indexOf("data-desktop-session-actions");
  const end = topBarSource.indexOf("<SessionReloadDialog", start);
  const actions = topBarSource.slice(start, end);

  assert(start >= 0);
  assert(end > start);
  assert(actions.includes("desktopSessionActionSx"));
  // Both tiers are pinned here: the full widths, and the single icon width the
  // cluster falls back to when the toolbar cannot afford the words. Density is
  // the ONLY thing that may change them — a control must never be sized by
  // whether it happens to fit.
  assert(actions.includes('minWidth: density === "full" ? 90'));
  assert(actions.includes("ACTION_ICON_WIDTH_PX"));
  // Collapsing the word may not cost the action its name or its letter.
  assert(actions.includes('aria-label="Reload session runtime"'));
  assert(actions.includes('keyLabel="L"'));
  assertEquals(actions.includes("contextPercent"), false);
  assert(
    embeddedControlSource.includes("export function desktopSessionActionSx"),
  );
  assert(
    embeddedControlSource.includes(
      "export const DESKTOP_TOPBAR_CONTROL_HEIGHT = 38",
    ),
  );
  // The height follows the global font size (a desktopSize length).
  assert(
    embeddedControlSource.includes(
      "export const DESKTOP_TOPBAR_CONTROL_SIZE = desktopSize(DESKTOP_TOPBAR_CONTROL_HEIGHT)",
    ),
  );
  assert(
    embeddedControlSource.includes("height: DESKTOP_TOPBAR_CONTROL_SIZE"),
  );
});

test("desktop top-bar controls share height and spacing vocabulary", () => {
  assert(topBarSource.includes("DESKTOP_TOPBAR_CONTROL_SIZE"));
  assertEquals(topBarSource.includes("height: DESKTOP_TOPBAR_CONTROL_HEIGHT"), false);
  assert(topBarSource.includes("DESKTOP_TOPBAR_CONTROL_GAP"));
  assert(topBarSource.includes("DESKTOP_TOPBAR_CONTROL_GAP_PX"));
  assert(topBarSource.includes("spacing={DESKTOP_TOPBAR_CONTROL_GAP}"));
});

test("desktop top-bar surfaces have one mutually exclusive owner", () => {
  assert(
    topBarSource.includes(
      '"config" | "usage" | "update" | "reload" | null',
    ),
  );
  assert(topBarSource.includes("const configOpen = openSurface === \"config\""));
  assert(topBarSource.includes("const usageOpen = openSurface === \"usage\""));
  assert(
    topBarSource.includes(
      'const updateTarget = openSurface === "update" ? session ?? null : null',
    ),
  );
  assertEquals(topBarSource.includes("useState<HTMLElement | null>(null)"), false);
  assertEquals(topBarSource.includes("setCompactConfirm"), false);
  assertEquals(topBarSource.includes("setClearConfirm"), false);
});

test("top-bar shortcut availability does not select every control", () => {
  assertEquals(topBarSource.includes("active: shortcutsActive"), false);
  assertEquals(topBarSource.includes("accent={shortcutsActive ||"), false);
  assertEquals(composerSource.includes("desktopShortcutActive"), false);
});

test("desktop Stop holds its slot while idle", () => {
  const start = toolbarSource.indexOf('id: "stop"');
  const stop = toolbarSource.slice(start, toolbarSource.indexOf("})}", start));
  assert(start >= 0);
  // Idle Stop keeps its width invisibly; it appears, red, while a turn runs.
  assert(stop.includes("held: !session.stop.busy"));
  assert(toolbarSource.includes('...(held && { visibility: "hidden" })'));
  assert(stop.includes('tone: "error"'));
  assertEquals(toolbarSource.includes("session.stop.busy && action("), false);
});

test("desktop composer no longer owns the icon-only Clear action", () => {
  assertEquals(
    composerSource.includes(
      'size="small"\n                    aria-label="clear conversation"',
    ),
    false,
  );
  assertEquals(
    composerSource.includes(
      'aria-label="clear conversation from session settings"',
    ),
    true,
  );
});

test("desktop Provider update is a top-bar action only while offered", () => {
  assert(topBarSource.includes("const providerUpdate = sessionProviderUpdate(session)"));
  assert(topBarSource.includes("{providerUpdate && session && ("));
  assert(topBarSource.includes('data-desktop-topbar-action="update"'));
  assert(topBarSource.includes('sequence: [DESKTOP_WORKSPACE_PREFIX, "T", "N"]'));
  assert(topBarSource.includes("<ProviderUpdateDialog"));
});
