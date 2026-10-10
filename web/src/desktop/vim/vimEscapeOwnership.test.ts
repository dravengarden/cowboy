import { test } from "bun:test";
import { assertEquals } from "@std/assert";

import { vimEscapeBelongsToApp } from "./vimEscapeOwnership.ts";

test("Cowboy owns Escape directly when Vim is disabled", () => {
  assertEquals(vimEscapeBelongsToApp(false, undefined), true);
});

test("only plain Vim Normal mode delegates Escape to Cowboy", () => {
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

test("Vim keeps Escape while its runtime is still attaching", () => {
  assertEquals(vimEscapeBelongsToApp(true, undefined), false);
});
