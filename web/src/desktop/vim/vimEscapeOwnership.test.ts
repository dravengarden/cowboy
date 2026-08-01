import { assertEquals } from "jsr:@std/assert";

import { vimEscapeBelongsToApp } from "./vimEscapeOwnership.ts";

Deno.test("Cowboy owns Escape directly when Vim is disabled", () => {
  assertEquals(vimEscapeBelongsToApp(false, undefined), true);
});

Deno.test("only plain Vim Normal mode delegates Escape to Cowboy", () => {
  assertEquals(vimEscapeBelongsToApp(true, {}), true);
  assertEquals(vimEscapeBelongsToApp(true, { insertMode: true }), false);
  assertEquals(vimEscapeBelongsToApp(true, { visualMode: true }), false);
  assertEquals(
    vimEscapeBelongsToApp(true, { inputState: { operator: "change" } }),
    false,
  );
  assertEquals(
    vimEscapeBelongsToApp(true, { inputState: { keyBuffer: ["g"] } }),
    false,
  );
});

Deno.test("Vim keeps Escape while its runtime is still attaching", () => {
  assertEquals(vimEscapeBelongsToApp(true, undefined), false);
});
