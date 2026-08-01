import { assertEquals } from "jsr:@std/assert";
import {
  retainTranscriptSessionCache,
  touchTranscriptSessionCache,
} from "./transcriptSessionCache";

Deno.test("transcript session cache keeps the current session and evicts LRU history", () => {
  let order: string[] = [];
  for (const id of ["a", "b", "c", "d"]) {
    order = touchTranscriptSessionCache(order, id, 3).order;
  }
  assertEquals(order, ["b", "c", "d"]);

  const revisited = touchTranscriptSessionCache(order, "b", 3);
  assertEquals(revisited.order, ["c", "d", "b"]);
  assertEquals(revisited.evicted, []);

  const opened = touchTranscriptSessionCache(revisited.order, "e", 3);
  assertEquals(opened.order, ["d", "b", "e"]);
  assertEquals(opened.evicted, ["c"]);
});

Deno.test("transcript session cache drops deleted sessions", () => {
  const retained = retainTranscriptSessionCache(
    ["a", "b", "c"],
    new Set(["a", "c"]),
  );
  assertEquals(retained.order, ["a", "c"]);
  assertEquals(retained.evicted, ["b"]);
});
