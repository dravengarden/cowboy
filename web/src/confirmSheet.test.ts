import { assert, assertEquals } from "jsr:@std/assert";

const sheetSource = await Deno.readTextFile(
  new URL("./Sheet.tsx", import.meta.url),
);
const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const reloadSource = await Deno.readTextFile(
  new URL("./SessionReloadDialog.tsx", import.meta.url),
);
const fullscreenSource = await Deno.readTextFile(
  new URL("./FullscreenComposer.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const infoSource = await Deno.readTextFile(
  new URL("./InfoSheet.tsx", import.meta.url),
);
const providerSource = await Deno.readTextFile(
  new URL("./ProviderManagement.tsx", import.meta.url),
);
const desktopTopBarSource = await Deno.readTextFile(
  new URL("./desktop/DesktopTopBarControls.tsx", import.meta.url),
);

Deno.test("ConfirmSheet forces the compact Obsidian card on mobile and tablet", () => {
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
    sheetSource.includes(
      'mobileDismiss={useDock || actions != null ? "none" : "footer"}',
    ),
  );
  assert(sheetSource.includes("<MobileDecisionDock"));
  assert(sheetSource.includes("dockClearance={useDock}"));
});

Deno.test("phone-facing confirms supply a mobile decision dock", () => {
  assert(
    composerSource.includes(
      'confirmLabel: action.destructive ? "Clear" : "Compact"',
    ),
  );
  assert(composerSource.includes('cancelLabel: "Keep running"'));
  assert(reloadSource.includes('confirmLabel: "Reload"'));
  assert(fullscreenSource.includes('cancelLabel: "Keep editing"'));
  assert(appSource.includes('confirmLabel: "Update and roll out"'));
  assert(infoSource.includes('confirmLabel: resetMode === "schedule"'));
  assert(infoSource.includes('"Schedule reset"'));
  assert(infoSource.includes('"Reset now"'));
  assert(
    providerSource.includes('confirmLabel: "Uninstall and remove sessions"'),
  );
});

Deno.test("phone-facing confirmation prompts use ConfirmSheet, not a raw Dialog", () => {
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

Deno.test("desktop-owned session confirms stay centered dialogs", () => {
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
