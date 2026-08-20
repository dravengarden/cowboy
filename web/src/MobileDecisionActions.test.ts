import { assertEquals } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("./MobileDecisionActions.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("mobile decisions use a shared labeled action footer", () => {
  assertEquals(source.includes("export function MobileDecisionActions("), true);
  assertEquals(source.includes("data-mobile-decision-actions"), true);
  assertEquals(source.includes('variant="contained"'), true);
  assertEquals(source.includes('justifyContent: "space-between"'), true);
  assertEquals(source.includes("MobileSheetActionGroup"), false);
  assertEquals(source.includes("createPortal"), false);
});

Deno.test("mobile new session uses labeled Cancel and Create actions", () => {
  const dialog = appSource.slice(
    appSource.indexOf("function NewSessionDialog("),
    appSource.indexOf("const EMPTY_TRANSCRIPT_TIMELINE"),
  );
  assertEquals(dialog.includes("<MobileDecisionActions"), true);
  assertEquals(
    dialog.includes('confirmLabel={creating ? "Creating…" : "Create"}'),
    true,
  );
  assertEquals(dialog.includes("footerOverlay"), false);
  assertEquals(dialog.includes("data-new-session-footer-actions"), true);
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
  assertEquals(settings.includes('confirmLabel="Save"'), true);
  assertEquals(settings.includes("onCancel={cancelTitle}"), true);
  assertEquals(settings.includes("preserveFocus"), true);
  assertEquals(settings.includes("MobileDecisionDock"), false);
});
