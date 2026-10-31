import { expect, test } from "bun:test";
import {
  duplicateMode,
  duplicateSession,
  forkAvailability,
  forkSession,
} from "./sessionFork";

const machines = [
  { id: "ovh", capabilities: { hibernation: true, native_fork: true } },
  { id: "old", capabilities: { hibernation: true } },
];

test("Fork is offered only for verified Providers on a fork-capable Machine", () => {
  const session = {
    provider: "claude-code",
    status: "running",
    machine_id: "ovh",
    system: false,
  } as const;
  expect(forkAvailability(session, machines)).toBe("ready");
  expect(forkAvailability({ ...session, status: "busy" }, machines)).toBe(
    "busy",
  );
  expect(forkAvailability({ ...session, provider: "codex" }, machines)).toBe(
    "ready",
  );
  expect(forkAvailability({ ...session, provider: "grok" }, machines)).toBe(
    null,
  );
  expect(forkAvailability({ ...session, machine_id: "old" }, machines)).toBe(
    null,
  );
  expect(forkAvailability({ ...session, system: true }, machines)).toBe(null);
});

test("forkSession posts one operation and returns the destination", async () => {
  const calls: { url: string; body: unknown }[] = [];
  const receipt = await forkSession(
    "sess/1",
    { messageId: "msg_1", operation: "fork-op" },
    (url, init) => {
      calls.push({ url, body: JSON.parse(String(init.body)) });
      return Promise.resolve(
        new Response(
          JSON.stringify({ session_id: "sess-2", session: { id: "sess-2" } }),
          { status: 201 },
        ),
      );
    },
  );
  expect(calls).toEqual([{
    url: "/api/sessions/sess%2F1/fork",
    body: { operation: "fork-op", message_id: "msg_1" },
  }]);
  expect(receipt.session_id).toBe("sess-2");
});

test("forkSession surfaces the server's refusal", async () => {
  const refused = forkSession(
    "sess-1",
    {},
    () =>
      Promise.resolve(
        new Response(JSON.stringify({ error: "Wait for this agent" }), {
          status: 409,
        }),
      ),
  );
  expect(refused).rejects.toThrow("Wait for this agent");
});

test("Duplicate forks natively only on the same Provider and subscription", () => {
  const base = {
    sourceProvider: "claude-code",
    sourceAccount: "default",
    provider: "claude-code",
    account: "default",
    forkAvailable: true,
  };
  expect(duplicateMode(base)).toBe("fork");
  expect(duplicateMode({ ...base, account: "mink" })).toBe("linked");
  expect(duplicateMode({ ...base, provider: "codex" })).toBe("linked");
  expect(duplicateMode({ ...base, forkAvailable: false })).toBe("linked");
});

test("Duplicate requests carry config overrides and an idempotency key", async () => {
  const seen: { url: string; init: RequestInit }[] = [];
  const fetcher = ((url: string, init: RequestInit) => {
    seen.push({ url, init });
    return Promise.resolve(Response.json({ session_id: "copy", session: null }, { status: 201 }));
  }) as unknown as typeof fetch;
  await forkSession("s", { operation: "op", config: { model: "opus" } }, fetcher);
  expect(JSON.parse(String(seen[0]!.init.body))).toEqual({
    operation: "op",
    config: { model: "opus" },
  });
  const receipt = await duplicateSession(
    "s",
    { provider: "codex", provider_account_id: "pika", preset: "codex/max" },
    "key-1",
    fetcher,
  );
  expect(receipt.session_id).toBe("copy");
  expect(seen[1]!.url).toBe("/api/sessions/s/duplicate");
  expect((seen[1]!.init.headers as Record<string, string>)["idempotency-key"]).toBe("key-1");
  expect(JSON.parse(String(seen[1]!.init.body))).toEqual({
    operation: "key-1",
    provider: "codex",
    provider_account_id: "pika",
    preset: "codex/max",
  });
});
