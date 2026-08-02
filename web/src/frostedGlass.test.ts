import { assertEquals } from "jsr:@std/assert";
import {
  FROSTED_PILL_DROP_SHADOW_GEOMETRY,
  FLOATING_OVERLAY_BOUNDARY_GAP_PX,
} from "./floatingOverlayPolicy";

Deno.test("floating composer overlays keep a narrow boundary seam", () => {
  assertEquals(FLOATING_OVERLAY_BOUNDARY_GAP_PX, 4);
});

Deno.test("frosted pill elevation stays close to the floating surface", () => {
  assertEquals(FROSTED_PILL_DROP_SHADOW_GEOMETRY, "0 5px 16px -10px");
});
