import { assertEquals } from "jsr:@std/assert";
import { classifySurface } from "./profile";

Deno.test("desktop host stays desktop with touch capability", () => {
  assertEquals(
    classifySurface({
      finePointer: true,
      hover: true,
      tabletWidth: true,
      touchCapable: true,
      desktopHost: true,
    }).kind,
    "desktop",
  );
});

Deno.test("iPad with a trackpad remains a touch tablet", () => {
  assertEquals(
    classifySurface({
      finePointer: true,
      hover: true,
      tabletWidth: true,
      touchCapable: true,
      desktopHost: false,
    }),
    {
      kind: "tablet",
      input: "touch",
      touchCapable: true,
      finePointer: true,
      hover: true,
    },
  );
});

Deno.test("narrow touch surface is mobile", () => {
  assertEquals(
    classifySurface({
      finePointer: false,
      hover: false,
      tabletWidth: false,
      touchCapable: true,
      desktopHost: false,
    }).kind,
    "mobile",
  );
});
