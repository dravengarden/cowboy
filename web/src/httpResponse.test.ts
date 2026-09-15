import { assertEquals, assertRejects } from "jsr:@std/assert";
import { expectHttpOk } from "./httpResponse.ts";

Deno.test("HTTP failures keep the action, status and plaintext server reason", async () => {
  const error = await assertRejects(
    () =>
      expectHttpOk(
        new Response(" unknown usage provider\n", { status: 400 }),
        "Could not refresh usage",
      ),
  );
  assertEquals(
    error.message,
    "Could not refresh usage (HTTP 400): unknown usage provider",
  );
});

for (
  const body of [
    { detail: "Refresh is unavailable" },
    { message: "Refresh is unavailable" },
    { error: "Refresh is unavailable" },
    { error: { message: "Refresh is unavailable" } },
    "Refresh is unavailable",
  ]
) {
  Deno.test(`HTTP errors extract structured explanation: ${JSON.stringify(body)}`, async () => {
    const error = await assertRejects(
      () =>
        expectHttpOk(
          Response.json(body, { status: 503 }),
          "Could not refresh usage",
        ),
    );
    assertEquals(
      error.message,
      "Could not refresh usage (HTTP 503): Refresh is unavailable",
    );
  });
}

Deno.test("empty, unrecognized JSON and proxy HTML errors still identify the operation", async () => {
  for (
    const body of [
      "",
      "null",
      '{"code":400}',
      "<!doctype html><h1>Proxy failure</h1>",
    ]
  ) {
    const error = await assertRejects(
      () =>
        expectHttpOk(
          new Response(body, { status: 400, statusText: "Bad Request" }),
          "Could not load usage",
        ),
    );
    assertEquals(error.message, "Could not load usage (HTTP 400): Bad Request");
  }
  const error = await assertRejects(
    () =>
      expectHttpOk(new Response(null, { status: 502 }), "Could not load usage"),
  );
  assertEquals(
    error.message,
    "Could not load usage (HTTP 502): The server returned no error details.",
  );
});

Deno.test("failed error-body reads keep the status and cancellation stays cancellation", async () => {
  const brokenBody = new ReadableStream({
    start(controller) {
      controller.error(new Error("lost body"));
    },
  });
  await assertRejects(
    () =>
      expectHttpOk(
        new Response(brokenBody, { status: 502 }),
        "Could not load usage",
      ),
    Error,
    "Could not load usage (HTTP 502)",
  );
  const aborted = new DOMException("Cancelled", "AbortError");
  const abortedBody = new ReadableStream({
    start(controller) {
      controller.error(aborted);
    },
  });
  const error = await assertRejects(
    () =>
      expectHttpOk(
        new Response(abortedBody, { status: 400 }),
        "Could not load usage",
      ),
  );
  assertEquals(error, aborted);
});

Deno.test("successful HTTP checks leave the body readable", async () => {
  const response = Response.json({ providers: [] });
  await expectHttpOk(response, "Could not load usage");
  assertEquals(await response.json(), { providers: [] });
});
