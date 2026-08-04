import { assertEquals } from "jsr:@std/assert";
import { desktopEmbeddedControlIconSx } from "./DesktopEmbeddedIcon.ts";

Deno.test("desktop embedded control icons follow the global root font size", () => {
  assertEquals(desktopEmbeddedControlIconSx(), {
    fontSize: "calc(20px * var(--cowboy-font-scale, 1))",
    width: "calc(20px * var(--cowboy-font-scale, 1))",
    height: "calc(20px * var(--cowboy-font-scale, 1))",
    flexShrink: 0,
  });
});
