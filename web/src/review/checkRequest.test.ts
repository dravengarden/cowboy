import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { sendCheckRequest } from "./checkRequest";

test("uncertain replies retain identity, explicit refusals and changed epochs permit a new explicit request", async () => {
  const request = { operation: "op-1", epoch: "epoch-1" };
  for (const [status, code, forget] of [
    ["accepted", 200, true],
    ["existing", 200, true],
    ["refused", 503, true],
    ["epoch_changed", 409, true],
    ["unknown", 409, false],
  ] as const) {
    const fetcher = (async (_url, init) => {
      assertEquals(JSON.parse(String(init?.body)), { check: "lint", ...request });
      return Response.json({ status, reason: "detail" }, { status: code });
    }) as typeof fetch;
    assertEquals((await sendCheckRequest("session", "lint", request, fetcher)).forget, forget);
  }
  const broken = (async () => {
    throw new Error("lost reply");
  }) as unknown as typeof fetch;
  assertEquals((await sendCheckRequest("s", "lint", request, broken)).forget, false);
});
