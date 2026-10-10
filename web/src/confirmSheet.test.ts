import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const sheetSource = await readFile(
  new URL("./Sheet.tsx", import.meta.url), "utf8",
);
const composerSource = await readFile(
  new URL("./Composer.tsx", import.meta.url), "utf8",
);
const reloadSource = await readFile(
  new URL("./SessionReloadDialog.tsx", import.meta.url), "utf8",
);
const fullscreenSource = await readFile(
  new URL("./FullscreenComposer.tsx", import.meta.url), "utf8",
);
const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);
const infoSource = await readFile(
  new URL("./InfoSheet.tsx", import.meta.url), "utf8",
);
const providerSource = await readFile(
  new URL("./ProviderManagement.tsx", import.meta.url), "utf8",
);
const desktopTopBarSource = await readFile(
  new URL("./desktop/DesktopTopBarControls.tsx", import.meta.url), "utf8",
);

test("ConfirmSheet forces the compact Obsidian card on mobile and tablet", () => {
  assert(sheetSource.includes("export function ConfirmSheet("));
  assert(sheetSource.includes("export function useConfirmSheetSurface("));
  assert(
    sheetSource.includes('return useSurfaceProfile().kind !== "desktop"'),
  );
  assert(sheetSource.includes("forceSheet={forceSheet}"));
  assert(sheetSource.includes("portal"));
  assert(sheetSource.includes("<ObsidianSheet"));
  assert(sheetSource.includes("useCompactCard"));
  assert(
    sheetSource.includes('mobileDismiss={actions == null ? "footer" : "none"}'),
  );
});

test("phone-facing confirmation prompts use ConfirmSheet, not a raw Dialog", () => {
  const phoneFacing = [
    composerSource,
    reloadSource,
    fullscreenSource,
    appSource,
    infoSource,
    providerSource,
  ];
  for (const source of phoneFacing) {
    assert(source.includes("<ConfirmSheet"));
    assertEquals(source.includes("<Dialog\n"), false);
    assertEquals(source.includes("<Dialog "), false);
  }
  assert(composerSource.includes('title="Stop the running turn?"'));
  assert(
    composerSource.includes("title={action !== null ? `${action.label}?`"),
  );
  assert(reloadSource.includes('title="Reload this session?"'));
  assert(fullscreenSource.includes('title="Ignore modifications?"'));
  assert(appSource.includes('title="Roll out this update?"'));
  assert(infoSource.includes("Use nearest reset now?"));
  assert(providerSource.includes("Uninstall ${"));
});

test("desktop-owned session confirms stay centered dialogs", () => {
  assert(
    desktopTopBarSource.includes(
      "<DialogTitle>Clear conversation?</DialogTitle>",
    ),
  );
  assert(
    desktopTopBarSource.includes(
      "<DialogTitle>Compact conversation?</DialogTitle>",
    ),
  );
  assertEquals(desktopTopBarSource.includes("<ConfirmSheet"), false);
});
