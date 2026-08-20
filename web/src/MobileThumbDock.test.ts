import { assertEquals } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("./MobileThumbDock.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("thumb dock pins islands to the visual viewport corners", () => {
  assertEquals(source.includes('data-mobile-thumb-dock="left"'), true);
  assertEquals(source.includes('data-mobile-thumb-dock="right"'), true);
  assertEquals(source.includes("translateY(-100%)"), true);
  assertEquals(source.includes("var(--vv-height, 100dvh)"), true);
  assertEquals(
    source.includes("createPortal(dock, globalThis.document.body)"),
    true,
  );
  assertEquals(source.includes('height: "var(--vv-height, 100dvh)"'), false);
});

Deno.test("mobile new session keeps its actions in the non-overlay sheet footer", () => {
  const dialog = appSource.slice(
    appSource.indexOf("function NewSessionDialog("),
    appSource.indexOf("const EMPTY_TRANSCRIPT_TIMELINE"),
  );
  assertEquals(dialog.includes("<MobileDecisionDock"), false);
  assertEquals(dialog.includes("footerOverlay"), false);
  assertEquals(dialog.includes("data-new-session-footer-actions"), true);
  assertEquals(dialog.includes("data-new-session-sticky-actions"), false);
  assertEquals(dialog.includes('position: "sticky"'), false);
  assertEquals(dialog.includes("footer={"), true);
  assertEquals(dialog.includes("SHEET_THUMB_CLEARANCE"), false);
});

Deno.test("decision dock is the shared cancel/confirm island pair", () => {
  assertEquals(source.includes("export function MobileDecisionDock("), true);
  assertEquals(source.includes('key: "cancel"'), true);
  assertEquals(source.includes('key: "confirm"'), true);
  assertEquals(source.includes("export const SHEET_THUMB_CLEARANCE"), true);
  assertEquals(source.includes("onPointerDownCapture={preserveInput}"), true);
});

Deno.test("session title editing uses corner Cancel and Save islands", async () => {
  const composerSource = await Deno.readTextFile(
    new URL("./Composer.tsx", import.meta.url),
  );
  const settings = composerSource.slice(
    composerSource.indexOf("function ComposerSheet("),
    composerSource.indexOf("function SessionInfoSection("),
  );
  assertEquals(settings.includes("<MobileDecisionDock"), true);
  assertEquals(settings.includes('confirmLabel="Save"'), true);
  assertEquals(settings.includes("onCancel={cancelTitle}"), true);
  assertEquals(settings.includes("preserveFocus"), true);
  assertEquals(settings.includes('aria-label="save session title"'), false);
});

Deno.test("compact confirm sheets keep Cancel and the labeled action", () => {
  const deleteShell = appSource.slice(
    appSource.indexOf("function DeleteSessionShell("),
    appSource.indexOf("function LoadingState("),
  );
  const renameShell = appSource.slice(
    appSource.indexOf("function RenameSessionShell("),
    appSource.indexOf("// --- Info:"),
  );
  assertEquals(deleteShell.includes("<MobileDecisionDock"), false);
  assertEquals(renameShell.includes("<MobileDecisionDock"), false);
  assertEquals(deleteShell.includes("Cancel"), true);
  assertEquals(renameShell.includes("Save"), true);
});
