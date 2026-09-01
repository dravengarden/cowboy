import { assertEquals } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("./MobileDecisionActions.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const mobileProductShellSource = await Deno.readTextFile(
  new URL("./mobile/shell/MobileProductShell.tsx", import.meta.url),
);

Deno.test("mobile decisions use a shared labeled action footer", () => {
  assertEquals(source.includes("export function MobileDecisionActions("), true);
  assertEquals(source.includes("data-mobile-decision-actions"), true);
  assertEquals(source.includes("data-mobile-decision-footer-shelf"), true);
  assertEquals(source.includes('variant="contained"'), true);
  assertEquals(source.includes('justifyContent: "space-between"'), true);
  assertEquals(source.includes('width: "calc(100% + 32px)"'), true);
  assertEquals(source.includes("mx: -2"), true);
  assertEquals(source.includes("MobileSheetActionGroup"), false);
  assertEquals(source.includes("createPortal"), false);
});

Deno.test("mobile new session uses labeled Cancel and Create actions", () => {
  const dialog = appSource.slice(
    appSource.indexOf("function NewSessionDialog("),
    appSource.indexOf("const EMPTY_TRANSCRIPT_TIMELINE"),
  );
  assertEquals(dialog.includes("<MobileDecisionActions"), true);
  assertEquals(dialog.includes("shelf"), true);
  assertEquals(
    dialog.includes('confirmLabel={creating ? "Creating…" : "Create"}'),
    true,
  );
  assertEquals(dialog.includes("footerOverlay"), false);
  assertEquals(dialog.includes("data-new-session-footer-actions"), true);
  assertEquals(
    dialog.includes('data-new-session-footer-actions sx={{ width: "100%" }}'),
    true,
  );
  assertEquals(dialog.includes("0 -8px 18px"), false);
  assertEquals(
    mobileProductShellSource.includes(
      "[data-detent-sheet='true'][aria-label='New session']) [data-mobile-overflow-layer='true']",
    ),
    true,
  );
  assertEquals(
    mobileProductShellSource.includes(
      "[data-detent-sheet='true'][aria-label='New session']) [data-detent-sheet='true'] [data-mobile-overflow-layer='true']",
    ),
    true,
  );
});

Deno.test("session title editing uses labeled Cancel and Save actions", async () => {
  const composerSource = await Deno.readTextFile(
    new URL("./Composer.tsx", import.meta.url),
  );
  const settings = composerSource.slice(
    composerSource.indexOf("function ComposerSheet("),
    composerSource.indexOf("function SessionInfoSection("),
  );
  assertEquals(settings.includes("<MobileDecisionActions"), true);
  assertEquals(settings.includes("shelf"), true);
  assertEquals(
    settings.includes("floatingActions={useSheetSurface ? !editingTitle : true}"),
    true,
  );
  assertEquals(settings.includes('confirmLabel="Save"'), true);
  assertEquals(settings.includes("onCancel={cancelTitle}"), true);
  assertEquals(settings.includes("onConfirm={saveTitleAndClose}"), true);
  assertEquals(settings.includes("const saveTitleAndClose = (): void => {"), true);
  assertEquals(settings.includes("saveTitle();"), true);
  assertEquals(settings.includes("setCmdConfirm(null);\n    onClose();"), true);
  assertEquals(settings.includes("preserveFocus"), true);
  assertEquals(settings.includes("MobileDecisionDock"), false);
});
