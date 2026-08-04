import { assertEquals } from "jsr:@std/assert";
import { desktopEmbeddedControlIconSx } from "./DesktopEmbeddedIcon.ts";

Deno.test("desktop embedded control icons follow the global root font size", () => {
  assertEquals(desktopEmbeddedControlIconSx(), {
    fontSize: "1.25rem",
    flexShrink: 0,
  });
});
