import { assert, assertEquals } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const start = source.indexOf("{sortable.order.map((id, index) => {");
const end = source.indexOf("<Box sx={{ flex: 1, minWidth: 0 }}>", start);
const leading = source.slice(start, end);

Deno.test("pending row ordinal shares the reorder grip slot", () => {
  assert(start >= 0);
  assert(end > start);
  assert(leading.includes("const leadingHandle = editingId !== m.id && !optimistic && count > 1"));
  assert(leading.includes('aria-label="Drag to reorder"'));
  assert(leading.includes("width: 44"));
  assert(leading.includes("height: 44"));
  assert(leading.includes("<DesktopListJumpKeycap"));
  assert(leading.includes('position: "absolute"'));
  assert(
    leading.indexOf("<DesktopListJumpKeycap") >
      leading.indexOf('aria-label="Drag to reorder"'),
  );
});

Deno.test("pending row no longer gives the ordinal its own leading column", () => {
  assertEquals(
    leading.includes('alignSelf: "stretch",\n                          pt: 0.75'),
    false,
  );
});
