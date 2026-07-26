import { assertEquals } from "jsr:@std/assert";
import { shouldAdoptLoadedPage } from "./retainedPage.ts";

Deno.test("a retained unloaded page is not overwritten by the live tail", () => {
  assertEquals(shouldAdoptLoadedPage("older", "latest", ["latest"]), false);
});

Deno.test("an absent selection adopts the current loaded page", () => {
  assertEquals(shouldAdoptLoadedPage(null, "latest", ["latest"]), true);
});

Deno.test("a stale selection can resolve after its page is loaded", () => {
  assertEquals(
    shouldAdoptLoadedPage("older", "latest", ["older", "latest"]),
    true,
  );
});
