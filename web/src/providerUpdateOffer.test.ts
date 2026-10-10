import { assertEquals } from "jsr:@std/assert";
import type { SessionMeta } from "./protocol";
import {
  providerUpdateScheduleText,
  requestProviderUpdateWhenIdle,
  sessionProviderUpdate,
} from "./providerUpdateOffer";

const offer = { version: "3.4.10", digest: "new" };
const base: SessionMeta = {
  id: "s",
  provider: "claude-code",
  provider_version: "3.4.9",
  provider_generation_digest: "old",
  cwd: "/w",
  title: "t",
  status: "running",
  provider_update_available: offer,
};

Deno.test("offers only a different release outside updates and system sessions", () => {
  assertEquals(sessionProviderUpdate(base), offer);
  assertEquals(sessionProviderUpdate(undefined), null);
  assertEquals(sessionProviderUpdate({ ...base, system: true }), null);
  assertEquals(
    sessionProviderUpdate({
      ...base,
      provider_update: {
        from: "3.4.9",
        to: "3.4.10",
        automatic: false,
        started_at_ms: 0,
      },
    }),
    null,
  );
  assertEquals(
    sessionProviderUpdate({ ...base, provider_generation_digest: "new" }),
    null,
  );
});

Deno.test("schedule text explains what happens without action", () => {
  const now = 1_000_000;
  assertEquals(
    providerUpdateScheduleText({ ...offer, when_idle: true }, true, now),
    "after this turn",
  );
  assertEquals(providerUpdateScheduleText(offer, false, now), null);
  assertEquals(
    providerUpdateScheduleText(
      { ...offer, automatic_at_ms: now + 42 * 60_000 },
      false,
      now,
    ),
    "auto after ~42 min idle",
  );
  assertEquals(
    providerUpdateScheduleText(
      { ...offer, automatic_at_ms: now + 90 * 60_000 },
      false,
      now,
    ),
    "auto after ~1.5 h idle",
  );
  assertEquals(
    providerUpdateScheduleText({ ...offer, automatic_at_ms: now }, false, now),
    "auto when idle",
  );
});

Deno.test("the one-shot request is a session policy write", async () => {
  const calls: [string, RequestInit][] = [];
  await requestProviderUpdateWhenIdle("a/b", true, (input, init) => {
    calls.push([input, init]);
    return Promise.resolve(new Response("{}"));
  });
  assertEquals(calls[0]?.[0], "/api/sessions/a%2Fb/reload");
  assertEquals(calls[0]?.[1].method, "PUT");
  assertEquals(calls[0]?.[1].body, JSON.stringify({ when_idle: true }));
});
