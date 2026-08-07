import { assert, assertEquals } from "jsr:@std/assert";

const drawerSource = await Deno.readTextFile(
  new URL("./mobileSpatialDrawer.ts", import.meta.url),
);

Deno.test("mobile drawer keeps clipping and shadows off the heavy surface", () => {
  assertEquals(drawerSource.includes("surface.style.borderRadius"), false);
  assertEquals(drawerSource.includes("surface.style.boxShadow"), false);
  assert(drawerSource.includes("drawerMask.style.boxShadow"));
  assert(drawerSource.includes('surface.style.willChange = "transform"'));
});
