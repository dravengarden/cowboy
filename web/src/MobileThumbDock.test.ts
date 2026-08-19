import { assertEquals } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("./MobileThumbDock.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("thumb dock pins to the visual viewport box, not the sheet footer", () => {
  assertEquals(source.includes('top: "var(--vv-offset, 0px)"'), true);
  assertEquals(source.includes('height: "var(--vv-height, 100dvh)"'), true);
  assertEquals(source.includes('data-mobile-thumb-dock="true"'), true);
  assertEquals(source.includes('pb: keyboardOpen'), true);
  assertEquals(source.includes('createPortal(dock, globalThis.document.body)'), true);
});

Deno.test("mobile new session uses the thumb dock instead of a sheet footer", () => {
  const dialog = appSource.slice(
    appSource.indexOf("function NewSessionDialog("),
    appSource.indexOf("const EMPTY_TRANSCRIPT_TIMELINE"),
  );
  assertEquals(dialog.includes("<MobileThumbDock"), true);
  assertEquals(dialog.includes("footerOverlay"), false);
  assertEquals(dialog.includes('key: "cancel"'), true);
  assertEquals(dialog.includes('key: "create"'), true);
});
