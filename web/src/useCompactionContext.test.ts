import { assertEquals } from "jsr:@std/assert";
import { contextValueFromSessionInfo } from "./sessionInfoContext.ts";

Deno.test("session info context decoder reads flattened SessionMeta fields", () => {
  assertEquals(
    contextValueFromSessionInfo({
      id: "session-1",
      status: "running",
      context_used: 21_000,
      context_size: 353_400,
      event_count: 42,
    }),
    { used: 21_000, size: 353_400 },
  );
});

Deno.test("session info context decoder rejects the obsolete nested shape", () => {
  assertEquals(
    contextValueFromSessionInfo({
      meta: { context_used: 21_000, context_size: 353_400 },
    }),
    null,
  );
});

Deno.test("session info context decoder rejects incomplete values", () => {
  assertEquals(contextValueFromSessionInfo({ context_used: 21_000 }), null);
  assertEquals(
    contextValueFromSessionInfo({ context_used: -1, context_size: 353_400 }),
    null,
  );
});
