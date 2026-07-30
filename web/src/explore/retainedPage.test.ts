import { assertEquals, assertNotEquals } from "jsr:@std/assert";
import {
  pageStartHandshakeIdentity,
  shouldAdoptLoadedPage,
} from "./retainedPage.ts";

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

Deno.test("streamed answer chunks do not restart the page-start handshake", () => {
  const initial = pageStartHandshakeIdentity("question", ["question", "answer-1"]);
  const streamed = pageStartHandshakeIdentity("question", [
    "question",
    "answer-1",
    "answer-2",
  ]);
  assertEquals(streamed, initial);
});

Deno.test("a different page root starts a new page-start handshake", () => {
  assertNotEquals(
    pageStartHandshakeIdentity("next", ["next"]),
    pageStartHandshakeIdentity("question", ["question"]),
  );
});
