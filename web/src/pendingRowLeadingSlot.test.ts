import { assert, assertEquals } from "jsr:@std/assert";
import {
  mobileComposerIdleEditorMinHeight,
  mobileComposerPanelHeaderMinHeight,
  mobilePendingRowMinHeight,
} from "./mobileComposerPrimitives.ts";

const source = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const start = source.indexOf("{sortable.order.map((id, index) => {");
const end = source.indexOf("<Box sx={{ flex: 1, minWidth: 0 }}>", start);
const leading = source.slice(start, end);

Deno.test("pending row ordinal shares the reorder grip slot", () => {
  assert(start >= 0);
  assert(end > start);
  assert(
    /const leadingHandle =\s*editingId !== m\.id\s*&&\s*!optimistic\s*&&\s*count > 1/
      .test(leading),
  );
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

Deno.test("empty draft and queue cards match the compact composer card height", () => {
  assertEquals(
    mobilePendingRowMinHeight,
    mobileComposerIdleEditorMinHeight + mobileComposerPanelHeaderMinHeight,
  );
  assertEquals(mobilePendingRowMinHeight, 92);
  assert(source.includes("minHeight: mobilePendingRowMinHeight"));
  assert(source.includes("minHeight: mobileComposerIdleEditorMinHeight"));
  assertEquals(source.includes("minHeight: 38"), false);
});

Deno.test("pending row no longer gives the ordinal its own leading column", () => {
  assertEquals(
    leading.includes(
      'alignSelf: "stretch",\n                          pt: 0.75',
    ),
    false,
  );
});
