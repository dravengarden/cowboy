import { assert, assertEquals } from "jsr:@std/assert";

const topBarSource = await Deno.readTextFile(
  new URL("./DesktopTopBarControls.tsx", import.meta.url),
);
const composerSource = await Deno.readTextFile(
  new URL("../Composer.tsx", import.meta.url),
);
const embeddedControlSource = await Deno.readTextFile(
  new URL("./DesktopEmbeddedControl.ts", import.meta.url),
);

Deno.test("desktop session actions form one reload-to-stop cluster", () => {
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

Deno.test("desktop session actions share compact geometry without duplicate context text", () => {
  const start = topBarSource.indexOf("data-desktop-session-actions");
  const end = topBarSource.indexOf("<SessionReloadDialog", start);
  const actions = topBarSource.slice(start, end);

  assert(start >= 0);
  assert(end > start);
  assert(actions.includes("desktopSessionActionSx"));
  assert(actions.includes("minWidth: 90"));
  assert(actions.includes("minWidth: 96"));
  assert(actions.includes("minWidth: 80"));
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

Deno.test("desktop top-bar controls share height and spacing vocabulary", () => {
  assert(topBarSource.includes("DESKTOP_TOPBAR_CONTROL_HEIGHT"));
  assert(topBarSource.includes("DESKTOP_TOPBAR_CONTROL_GAP"));
  assert(topBarSource.includes("DESKTOP_TOPBAR_CONTROL_GAP_PX"));
  assert(topBarSource.includes("spacing={DESKTOP_TOPBAR_CONTROL_GAP}"));
});

Deno.test("desktop top-bar surfaces have one mutually exclusive owner", () => {
  assert(
    topBarSource.includes(
      '"config" | "usage" | "reload" | "compact" | "clear" | null',
    ),
  );
  assert(topBarSource.includes("const configOpen = openSurface === \"config\""));
  assert(topBarSource.includes("const usageOpen = openSurface === \"usage\""));
  assert(topBarSource.includes("const compactConfirm = openSurface === \"compact\""));
  assert(topBarSource.includes("const clearConfirm = openSurface === \"clear\""));
  assertEquals(topBarSource.includes("useState<HTMLElement | null>(null)"), false);
  assertEquals(topBarSource.includes("setCompactConfirm"), false);
  assertEquals(topBarSource.includes("setClearConfirm"), false);
});

Deno.test("top-bar shortcut availability does not select every control", () => {
  assertEquals(topBarSource.includes("active: shortcutsActive"), false);
  assertEquals(topBarSource.includes("accent={shortcutsActive ||"), false);
  assertEquals(composerSource.includes("desktopShortcutActive"), false);
});

Deno.test("desktop Stop remains mounted and becomes disabled while idle", () => {
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
  assertEquals(desktopStop.includes("const stopButton = busy"), false);
});

Deno.test("desktop composer no longer owns the icon-only Clear action", () => {
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
