import { assertEquals, assertRejects } from "jsr:@std/assert@1.0.19";
import {
  changeExtensionInstallation,
  extensionManagementJson,
} from "./managementApi.ts";

Deno.test("extension lifecycle accepts empty successful acknowledgments", async () => {
  const original = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = (_input, init) => {
    calls++;
    assertEquals(init?.method, "POST");
    assertEquals(init?.cache, "no-store");
    return Promise.resolve(new Response(null, { status: 204 }));
  };
  try {
    await changeExtensionInstallation("/install", { method: "POST" });
    await changeExtensionInstallation("/uninstall", { method: "POST" });
    assertEquals(calls, 2);
  } finally {
    globalThis.fetch = original;
  }
});

Deno.test("extension management preserves JSON plans and never retries an uncertain effect", async () => {
  const original = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = () => {
    calls++;
    return Promise.resolve(
      calls === 1
        ? Response.json({ plan_id: "original-plan" })
        : new Response("private diagnostic", { status: 503 }),
    );
  };
  try {
    assertEquals(await extensionManagementJson("/plan"), {
      plan_id: "original-plan",
    });
    await assertRejects(
      () => changeExtensionInstallation("/install", { method: "POST" }),
      Error,
      "Refresh installation status",
    );
    assertEquals(calls, 2);
  } finally {
    globalThis.fetch = original;
  }
});
