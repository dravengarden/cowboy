import { assertEquals } from "jsr:@std/assert";
import { updateDelayFromStorage } from "./clientUpdateSettings.ts";

Deno.test("update countdown defaults to three and preserves zero and custom seconds", () => {
  for (const value of ["", " ", "bad", "-1", "1.5", "3601", "Infinity"]) assertEquals(updateDelayFromStorage(value), 3);
  for (const value of [0, 3, 15, 3600]) assertEquals(updateDelayFromStorage(String(value)), value);
});
