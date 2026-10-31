import { expect, test } from "bun:test";
import { forkAvailability, forkSession } from "./sessionFork";

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
