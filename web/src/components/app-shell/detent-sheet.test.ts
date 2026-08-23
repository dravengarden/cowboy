import { assert, assertEquals } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("detent-sheet.tsx", import.meta.url),
);

Deno.test("drag paints the pointer sample without a frame of lag", () => {
  assert(source.includes('const SETTLE_EASE = "cubic-bezier(0.32, 0.72, 0, 1)"'));
  assert(source.includes("paint(y, false, true)"));
  assertEquals(source.includes("pendingYRef"), false);
  assert(
    source.includes("if (sheet.style.transition !== nextTransition)"),
  );
});

Deno.test("overlay footers own the exclusive hit strip over the scroll body", () => {
  const footer = source.slice(source.indexOf("data-detent-sheet-footer"));
  assert(footer.includes('data-detent-sheet-footer={footerOverlay ? "overlay" : "row"}'));
  assert(footer.includes("onPointerDown={(event) => event.stopPropagation()}"));
  assert(footer.includes("onClick={(event) => event.stopPropagation()}"));
  assertEquals(
    footer.slice(0, 800).includes('pointerEvents: "none"'),
    false,
  );
  assert(footer.includes('pointerEvents: topMost ? "auto" : "none"'));
});
