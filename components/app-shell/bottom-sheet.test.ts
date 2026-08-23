import { assert, assertEquals } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("bottom-sheet.tsx", import.meta.url),
);

Deno.test("touch dismiss consumes compatibility events before closing", () => {
  const down = source.slice(
    source.indexOf("const onPointerDown"),
    source.indexOf("const onPointerMove"),
  );
  const up = source.slice(
    source.indexOf("const onPointerUp"),
    source.indexOf("const onPointerCancel"),
  );
  const click = source.slice(
    source.indexOf("const onClick"),
    source.indexOf("return { onPointerDown"),
  );

  assert(down.indexOf("event.preventDefault()") < down.indexOf("startRef.current = {"));
  assert(up.indexOf("event.stopPropagation()") < up.indexOf("onClose()"));
  assert(up.indexOf("event.preventDefault()") < up.indexOf("onClose()"));
  assert(click.indexOf("event.stopPropagation()") < click.indexOf("onClose()"));
  assertEquals(source.includes("onPointerCancel: () => void"), false);
});

Deno.test("mobile sheet dismiss rim includes the whole centred tap target", () => {
  assert(source.includes("const MOBILE_SHEET_DISMISS_BUTTON_PX = 46;"));
  assert(source.includes("MOBILE_SHEET_DISMISS_BUTTON_PX + 2 * (4 + 1)"));
  assert(
    /width: MOBILE_SHEET_DISMISS_ISLAND_PX[\s\S]*MOBILE_SHEET_DISMISS_BUTTON_PX/u.test(source),
  );
});

Deno.test("overlay footer shields keep pointer events exclusive", () => {
  assert(source.includes('data-mobile-sheet-footer-shield'));
  assert(source.includes("swallowRetargetedClicks()"));
  assertEquals(source.includes('pointerEvents: "none"'), true);
  const dismiss = source.slice(source.indexOf("export function MobileSheetDismiss"));
  assert(dismiss.includes('pointerEvents: "auto"'));
  assertEquals(
    dismiss.slice(0, dismiss.indexOf("export function BottomSheet")).includes(
      'pointerEvents: "none"',
    ),
    false,
  );
});
