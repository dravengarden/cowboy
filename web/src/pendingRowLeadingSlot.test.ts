import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import {
  mobileComposerIdleEditorMinHeight,
  mobileComposerPanelHeaderMinHeight,
  mobilePendingRowMinHeight,
} from "./mobileComposerPrimitives.ts";

const source = await readFile(
  new URL("./Composer.tsx", import.meta.url), "utf8",
);
const start = source.indexOf("{sortable.order.map((id) => {");
const end = source.indexOf("<Box sx={{ flex: 1, minWidth: 0 }}>", start);
const leading = source.slice(start, end);

test("pending rows keep only the reorder grip in their leading slot", () => {
  assert(start >= 0);
  assert(end > start);
  assert(
    /const leadingHandle =\s*editingId !== m\.id\s*&&\s*!optimistic\s*&&\s*count > 1/
      .test(leading),
  );
  assert(leading.includes('aria-label="Drag to reorder"'));
  assert(leading.includes("const gripSize = 44"));
  // Desktop is keyboard-first: a slim edge handle, revealed on hover/focus,
  // carries the `O` reorder hint instead of a permanent 44px gutter.
  assert(leading.includes('className="cowboy-pending-grip"'));
  assert(leading.includes('badge="O"'));
  assert(leading.includes("width: gripSize"));
  assert(leading.includes("height: gripSize"));
  assert(leading.includes('position: "absolute"'));
  // Jumps use transient ' labels (FOCUS.md "Labels"); rows carry no ordinal.
  assertEquals(leading.includes("<DesktopListJumpKeycap"), false);
});

test("empty draft and queue cards match the compact composer card height", () => {
  assertEquals(
    mobilePendingRowMinHeight,
    mobileComposerIdleEditorMinHeight + mobileComposerPanelHeaderMinHeight,
  );
  assertEquals(mobilePendingRowMinHeight, 92);
  assert(source.includes("minHeight: mobilePendingRowMinHeight"));
  assert(source.includes("minHeight: mobileComposerIdleEditorMinHeight"));
  assertEquals(source.includes("minHeight: 38"), false);
});

test("pending row no longer gives the ordinal its own leading column", () => {
  assertEquals(
    leading.includes(
      'alignSelf: "stretch",\n                          pt: 0.75',
    ),
    false,
  );
});
