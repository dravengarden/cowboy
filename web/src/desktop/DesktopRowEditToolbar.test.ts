import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import { desktopToolbarActionSx } from "./desktopToolbarAction";

const rowEdit = await Bun.file(
  new URL("./DesktopRowEditToolbar.tsx", import.meta.url),
).text();
const composer = await Bun.file(
  new URL("../Composer.tsx", import.meta.url),
).text();

test("Desktop toolbar actions size every dimension from the root font", () => {
  const icon = desktopToolbarActionSx["& .MuiSvgIcon-root"];
  for (const value of [icon.fontSize, icon.width, icon.height]) {
    assertEquals(value.endsWith("rem"), true);
  }
  for (
    const value of [
      desktopToolbarActionSx.minHeight,
      desktopToolbarActionSx.px,
      desktopToolbarActionSx.gap,
    ]
  ) assertEquals(value.endsWith("rem"), true);
});

test("the Desktop row editor uses the prompt toolbar's action family", () => {
  assert(rowEdit.includes("sx={desktopToolbarActionSx}"));
  assert(rowEdit.includes("<LeaderKeycap"));
  // The touch bar's fixed 40px IconButtons and stacked keycaps stay Mobile-only.
  assert(composer.includes("<DesktopRowEditToolbar"));
  assert(composer.includes("{!desktop && <Stack"));
});
