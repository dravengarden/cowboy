import { test } from "bun:test";
import { assert } from "@std/assert";

const source = await Bun.file(
  new URL("./DesktopTopBarControls.tsx", import.meta.url),
).text();

test("run configuration hugs its summary and grows into top-bar slack", () => {
  // A viewport clamp cut "Bypass permissions · Opus …" off while the pane
  // still had room; the width now comes from the strip's measured slack.
  assert(!source.includes("18vw"));
  assert(source.includes("maxWidth: desktopSize(configMaxWidth)"));
  assert(source.includes("CONFIG_MIN_WIDTH_PX + Math.max("));
});

test("a session action keeps a gap between its word and keycap", () => {
  assert(source.includes('gap: "0.375rem", ...(density === "full"'));
  assert(!source.includes('ml: "auto !important"'));
});
