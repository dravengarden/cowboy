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
  assertEquals(source.includes('createPortal(dock, globalThis.document.body)'), true);
  assertEquals(source.includes('height: "var(--vv-height, 100dvh)"'), false);
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
