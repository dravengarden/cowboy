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

Deno.test("mobile new session uses the thumb dock instead of a sheet footer", () => {
  const dialog = appSource.slice(
    appSource.indexOf("function NewSessionDialog("),
    appSource.indexOf("const EMPTY_TRANSCRIPT_TIMELINE"),
  );
  assertEquals(dialog.includes("<MobileDecisionDock"), true);
  assertEquals(dialog.includes("footerOverlay"), false);
  assertEquals(dialog.includes("SHEET_THUMB_CLEARANCE"), true);
});

Deno.test("decision dock is the shared cancel/confirm island pair", () => {
  assertEquals(source.includes("export function MobileDecisionDock("), true);
  assertEquals(source.includes('key: "cancel"'), true);
  assertEquals(source.includes('key: "confirm"'), true);
  assertEquals(source.includes("export const SHEET_THUMB_CLEARANCE"), true);
});

Deno.test("delete rename and session info share the viewport dock", () => {
  assertEquals(appSource.includes('title="Delete this session?"'), true);
  assertEquals(appSource.includes("<MobileDecisionDock"), true);
  assertEquals(appSource.includes('confirmLabel="Delete"'), true);
  assertEquals(appSource.includes('confirmLabel="Save"'), true);
  assertEquals(appSource.includes('title="Session info"'), true);
  assertEquals(appSource.includes('label: "Close"'), true);
});
