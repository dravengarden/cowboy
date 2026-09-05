import { assertEquals, assertRejects } from "jsr:@std/assert";
import {
  loadSessionReloadPlan,
  reloadSession,
  type SessionReloadFetch,
} from "./sessionReload.ts";

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
      new Response("session workspace is still being prepared", {
        status: 409,
      }),
    );

  await assertRejects(
    () => reloadSession("s", {}, fetcher),
    Error,
    "session workspace is still being prepared",
  );
});

Deno.test("new Provider reload binds confirmation to the planned exact digest", async () => {
  let request = "";
  await reloadSession(
    "s",
    { providerGenerationDigest: "sha256:target" },
    (input) => {
      request = input;
      return Promise.resolve(new Response("reloading", { status: 202 }));
    },
  );
  assertEquals(
    request,
    "/api/sessions/s/reload?upgrade_provider=true&expected_generation_digest=sha256%3Atarget",
  );
});

Deno.test("reload plan reads installed version without mutating the session", async () => {
  const plan = {
    current_version: "1.1.2",
    target_version: "3.1.8",
    target_digest: "sha256:target",
    upgrade_available: true,
  };
  assertEquals(
    await loadSessionReloadPlan("s/a", (input, init) => {
      assertEquals(input, "/api/sessions/s%2Fa/reload");
      assertEquals(init.method, "GET");
      assertEquals(init.cache, "no-store");
      return Promise.resolve(Response.json(plan));
    }),
    plan,
  );
});

Deno.test("reload plan fails closed for old controllers and missing target identity", async () => {
  await assertRejects(
    () =>
      loadSessionReloadPlan(
        "s",
        () => Promise.resolve(new Response("", { status: 405 })),
      ),
    Error,
    "pinned version",
  );
  await assertRejects(
    () =>
      loadSessionReloadPlan("s", () =>
        Promise.resolve(
          Response.json({ current_version: "1", upgrade_available: true }),
        )),
    Error,
    "Invalid session reload plan",
  );
});
