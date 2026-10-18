import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { deliverCurrentTurn } from "./currentTurnDelivery";

test("a restored current-turn message only observes its exact receipt", async () => {
  const calls: Array<{ url: string; method: string }> = [];
  const request = (async (url: string, options?: RequestInit) => {
    calls.push({ url, method: options?.method ?? "GET" });
    return new Response("", { status: 404 });
  }) as typeof fetch;
  assertEquals(
    await deliverCurrentTurn("session", "original", null, request),
    "not_delivered",
  );
  assertEquals(calls, [{
    url: "/api/sessions/session/delivery/original",
    method: "GET",
  }]);
});

test("a lost native send reply is unknown, never a second POST or queued fallback", async () => {
  let calls = 0;
  const request = (async () => {
    calls++;
    throw new Error("lost reply");
  }) as typeof fetch;
  assertEquals(
    await deliverCurrentTurn(
      "s",
      "same",
      { text: "correction", content: [] },
      request,
    ),
    "unknown",
  );
  assertEquals(calls, 1);
});

test("native injection uses explicit delivery and stable identity", async () => {
  let body: unknown;
  const request = (async (_url: string, options?: RequestInit) => {
    body = JSON.parse(options?.body as string);
    return Response.json({ status: "injected" });
  }) as typeof fetch;
  assertEquals(
    await deliverCurrentTurn(
      "s",
      "same",
      { text: "correction", content: [] },
      request,
    ),
    "injected",
  );
  assertEquals(body, {
    text: "correction",
    content: [],
    delivery: "current_turn",
    operation: "same",
  });
});
