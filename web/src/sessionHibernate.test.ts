import { assertEquals, assertRejects } from "jsr:@std/assert";
import {
  hibernateAvailability,
  hibernateSession,
  type SessionHibernateFetch,
} from "./sessionHibernate.ts";

const ovh = { id: "ovh", capabilities: { hibernation: true } };

Deno.test("hibernation is offered only for a live session on a capable Machine", () => {
  assertEquals(hibernateAvailability({ status: "running", machine_id: "ovh" }, [ovh]), "ready");
  assertEquals(hibernateAvailability({ status: "busy", machine_id: "ovh" }, [ovh]), "busy");
  assertEquals(hibernateAvailability({ status: "exited", machine_id: "ovh" }, [ovh]), null);
  assertEquals(
    hibernateAvailability({ status: "running", machine_id: "ovh" }, [
      { id: "ovh", capabilities: { hibernation: false } },
    ]),
    null,
  );
  assertEquals(
    hibernateAvailability({ status: "running", machine_id: "ovh" }, [{ id: "ovh" }]),
    null,
  );
});

Deno.test("hibernation posts to the encoded session endpoint", async () => {
  let request: { input: string; init: RequestInit } | undefined;
  const fetcher: SessionHibernateFetch = (input, init) => {
    request = { input, init };
    return Promise.resolve(new Response("hibernating", { status: 202 }));
  };
  await hibernateSession("session/with spaces", fetcher);
  assertEquals(request?.input, "/api/sessions/session%2Fwith%20spaces/hibernate");
  assertEquals(request?.init.method, "POST");
});

Deno.test("a refused hibernation surfaces the Controller's reason", async () => {
  const fetcher: SessionHibernateFetch = () =>
    Promise.resolve(
      new Response("wait for the current turn to finish before hibernating", {
        status: 409,
      }),
    );
  await assertRejects(
    () => hibernateSession("s", fetcher),
    Error,
    "wait for the current turn to finish before hibernating",
  );
});
