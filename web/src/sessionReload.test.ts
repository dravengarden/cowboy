import { assertEquals, assertRejects } from "jsr:@std/assert";
import { reloadSession, type SessionReloadFetch } from "./sessionReload.ts";

Deno.test("session reload posts to the encoded session endpoint", async () => {
  let request: { input: string; init: RequestInit } | undefined;
  const fetcher: SessionReloadFetch = (input, init) => {
    request = { input, init };
    return Promise.resolve(new Response("reloading", { status: 202 }));
  };

  await reloadSession("session/with spaces", {}, fetcher);

  assertEquals(request?.input, "/api/sessions/session%2Fwith%20spaces/reload");
  assertEquals(request?.init.method, "POST");
});

Deno.test("session reload carries explicit active-turn confirmation", async () => {
  let request: { input: string; init: RequestInit } | undefined;
  const fetcher: SessionReloadFetch = (input, init) => {
    request = { input, init };
    return Promise.resolve(new Response("reloading", { status: 202 }));
  };

  await reloadSession("s", { confirmActiveTurn: true }, fetcher);

  assertEquals(
    request?.input,
    "/api/sessions/s/reload?confirm_active_turn=true",
  );
  assertEquals(request?.init.method, "POST");
});

Deno.test("session reload surfaces the backend rejection detail", async () => {
  const fetcher: SessionReloadFetch = () =>
    Promise.resolve(
      new Response("session workspace is still being prepared", { status: 409 }),
    );

  await assertRejects(
    () => reloadSession("s", {}, fetcher),
    Error,
    "session workspace is still being prepared",
  );
});
