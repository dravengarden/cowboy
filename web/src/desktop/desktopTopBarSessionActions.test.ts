import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const topBarSource = await readFile(
  new URL("./DesktopTopBarControls.tsx", import.meta.url), "utf8",
);
const composerSource = await readFile(
  new URL("../Composer.tsx", import.meta.url), "utf8",
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
  assert(
    shortcutDialogSource.includes(
      '{ keys: ["A"], title: "Verify Product Session when prompted in Top Bar" }',
    ),
  );
  assert(focusContract.includes("top-bar `R`/`U`/`A`/`L`/`C`/`X`"));
});

test("desktop session actions form one reload-to-stop cluster", () => {
  const reload = topBarSource.indexOf('data-desktop-item="topbar-reload"');
  const compact = topBarSource.indexOf('data-desktop-item="topbar-compact"');
  const clear = topBarSource.indexOf('data-desktop-item="topbar-clear"');
  const stop = topBarSource.indexOf("<AutoScrollAndStop");

  assert(reload >= 0);
  assert(compact > reload);
  assert(clear > compact);
  assert(stop > clear);
  assert(topBarSource.includes("<SessionReloadDialog"));
  assert(topBarSource.includes('keyLabel="L"'));
  assert(topBarSource.includes('shortcut: "L"'));
  assert(topBarSource.includes("data-desktop-clear"));
  assert(topBarSource.includes("<CleaningServices"));
  assert(topBarSource.includes('keyLabel="X"'));
  assert(topBarSource.includes('shortcut: "X"'));
  assert(topBarSource.includes("await resetSession(sessionId)"));
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
  assert(actions.includes('minWidth: density === "full" ? 96'));
  assert(actions.includes('minWidth: density === "full" ? 80'));
  assert(actions.includes("ACTION_ICON_WIDTH_PX"));
  // Collapsing the word may not cost the action its name or its letter.
  assert(actions.includes('aria-label="Reload session runtime"'));
  assert(actions.includes('aria-label="Clear conversation"'));
  assert(actions.includes('keyLabel="L"'));
  assert(actions.includes('keyLabel="C"'));
  assert(actions.includes('keyLabel="X"'));
  assertEquals(actions.includes("contextPercent"), false);
  assert(
    embeddedControlSource.includes("export function desktopSessionActionSx"),
  );
  assert(
    embeddedControlSource.includes(
      "export const DESKTOP_TOPBAR_CONTROL_HEIGHT = 38",
    ),
  );
  assert(
    embeddedControlSource.includes("height: DESKTOP_TOPBAR_CONTROL_HEIGHT"),
  );
});

test("desktop top-bar controls share height and spacing vocabulary", () => {
  assert(topBarSource.includes("DESKTOP_TOPBAR_CONTROL_HEIGHT"));
  assert(topBarSource.includes("DESKTOP_TOPBAR_CONTROL_GAP"));
  assert(topBarSource.includes("DESKTOP_TOPBAR_CONTROL_GAP_PX"));
  assert(topBarSource.includes("spacing={DESKTOP_TOPBAR_CONTROL_GAP}"));
});

test("desktop top-bar surfaces have one mutually exclusive owner", () => {
  assert(
    topBarSource.includes(
      '"config" | "usage" | "update" | "reload" | "compact" | "clear" | null',
    ),
  );
  assert(topBarSource.includes("const configOpen = openSurface === \"config\""));
  assert(topBarSource.includes("const usageOpen = openSurface === \"usage\""));
  assert(topBarSource.includes("const compactConfirm = openSurface === \"compact\""));
  assert(topBarSource.includes("const clearConfirm = openSurface === \"clear\""));
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

test("desktop Stop remains mounted and becomes disabled while idle", () => {
  const start = composerSource.indexOf(
    'if (presentation === "desktop-toolbar")',
  );
  const end = composerSource.indexOf("\n  return (", start);
  const desktopStop = composerSource.slice(start, end);

  assert(start >= 0);
  assert(end > start);
  assert(desktopStop.includes('data-desktop-topbar-action="stop"'));
  assert(desktopStop.includes("disabled={!busy}"));
  assert(
    /title=\{busy\s*\?\s*"Stop current turn"\s*:\s*"No turn is running"\}/
      .test(desktopStop),
  );
  assert(desktopStop.includes("desktopSessionActionSx"));
  // Stop follows the cluster density instead of always spending its word.
  assert(desktopStop.includes("collapsed ? ACTION_ICON_WIDTH_PX : 80"));
  assert(desktopStop.includes('aria-label="Stop current turn"'));
  assert(topBarSource.includes('collapsed={density !== "full"}'));
  assertEquals(desktopStop.includes("const stopButton = busy"), false);
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
  assert(topBarSource.includes('sequence: [DESKTOP_WORKSPACE_PREFIX, "T", "P"]'));
  assert(topBarSource.includes("<ProviderUpdateDialog"));
});
