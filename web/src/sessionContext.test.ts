import { afterEach, expect, test } from "bun:test";
import { derive } from "./derive";
import {
  announceContext,
  loadSessionContext,
  saveSessionContext,
  sendWithContext,
  withoutAgentSwitch,
  type SessionContext,
} from "./sessionContext";
import type { Envelope } from "./protocol";

const fetchOriginal = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = fetchOriginal;
});

test("plain HTTP failures allow a corrected send while successful handoffs leave the source usable", async () => {
  const context = {
    revision: 1,
    applied_revision: 0,
    sources: [],
    next_agent: null,
    delivery: null,
  };
  let status = 413;
  let posts = 0;
  globalThis.fetch = ((_input: RequestInfo | URL, init?: RequestInit) => {
    if (!init?.method) return Promise.resolve(Response.json(context));
    posts++;
    if (status !== 200) {
      return Promise.resolve(new Response("Payload too large", { status }));
    }
    return Promise.resolve(Response.json({ ...context, revision: 2 }));
  }) as typeof fetch;
  await loadSessionContext("plain-http-failure");
  await expect(sendWithContext("plain-http-failure", "large", [])).rejects
    .toThrow("too large");
  status = 403;
  await expect(sendWithContext("plain-http-failure", "corrected", [])).rejects
    .toThrow("HTTP 403");
  expect(posts).toBe(2);
  status = 200;
  await saveSessionContext("plain-http-failure", {
    revision: 1,
    sources: [],
    next_agent: null,
  });
  globalThis.fetch = (() =>
    Promise.resolve(Response.json({
      revision: 2,
      applied_revision: 2,
      sources: [],
      next_agent: null,
      delivery: {
        operation: "accepted",
        session_id: "different-session",
        state: "accepted",
      },
    }))) as typeof fetch;
  await expect(sendWithContext("plain-http-failure", "keep working here", []))
    .rejects.toThrow("previous message");
  expect(await sendWithContext("plain-http-failure", "keep working here", []))
    .toBe(false);
});

test("context bootstrap does not swallow or duplicate the user's message", () => {
  const reminder =
    '<system-reminder>\nLinked Cowboy sessions: [{"session_id":"old","before":9}]\nRead evidence.\n</system-reminder>';
  // The links follow the first dispatched authored prompt, which may be an
  // ordinary composer message when the queued bootstrap was removed.
  for (
    const [cmid, texts] of [
      ["context-once", [reminder, "Continue with this fix"]],
      ["context-once", [reminder + "Continue with this fix"]],
      ["c-composer", [reminder, "Continue with this fix"]],
      ["c-composer", [reminder + "Continue with this fix"]],
    ] as const
  ) {
    const timeline: Envelope[] = texts.map((text, index) => ({
      session_id: "new",
      seq: index + 1,
      kind: "update",
      cmid,
      update: {
        sessionUpdate: "user_message_chunk",
        content: { type: "text", text },
      },
    }));
    const items = derive(timeline);
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      kind: "message",
      role: "user",
      origin: { actor: "human" },
      chunks: [{ type: "text", text: "Continue with this fix" }],
    });
  }
});

test("ordinary sessions preserve the existing send path without extra HTTP", async () => {
  globalThis.fetch = (() => {
    throw new Error("Unexpected network request");
  }) as typeof fetch;
  expect(await sendWithContext("ordinary-no-context", "hello", [])).toBe(false);
});

test("concurrent context reads share one request", async () => {
  let reads = 0;
  globalThis.fetch = (() => {
    reads++;
    return Promise.resolve(Response.json({ revision: 0 }));
  }) as typeof fetch;
  await Promise.all([
    loadSessionContext("read-shared"),
    loadSessionContext("read-shared"),
  ]);
  expect(reads).toBe(1);
});

test("a change during an in-flight read cannot hide the newer revision", async () => {
  const previous = Promise.withResolvers<Response>();
  let reads = 0;
  globalThis.fetch = (() => {
    reads++;
    return reads === 1
      ? previous.promise
      : Promise.resolve(Response.json({ revision: 2 }));
  }) as typeof fetch;
  const old = loadSessionContext("read-race");
  announceContext("read-race");
  const current = loadSessionContext("read-race");
  previous.resolve(Response.json({ revision: 1 }));
  const snapshots = await Promise.all([old, current]);
  expect(snapshots.map((snapshot) => snapshot.revision)).toEqual([2, 2]);
});

test("unknown sends retain their operation and never send edited text", async () => {
  const context: SessionContext = {
    revision: 1,
    applied_revision: 0,
    sources: [],
    next_agent: null,
    source_sessions: [],
    max_sources: 32,
    delivery: null,
  };
  const requests: { operation: string; text: string }[] = [];
  globalThis.fetch = ((_input: RequestInfo | URL, init?: RequestInit) => {
    if (!init?.method) return Promise.resolve(Response.json(context));
    requests.push(JSON.parse(String(init.body)));
    return Promise.reject(new TypeError("Lost response"));
  }) as typeof fetch;
  await loadSessionContext("uncertain-send");
  await expect(sendWithContext("uncertain-send", "once", [])).rejects.toThrow(
    "Lost response",
  );
  await expect(sendWithContext("uncertain-send", "changed", [])).rejects
    .toThrow("unresolved");
  await expect(sendWithContext("uncertain-send", "once", [])).rejects.toThrow(
    "Lost response",
  );
  expect(requests).toHaveLength(2);
  expect(requests[0]!.operation).toBe(requests[1]!.operation);
});

test("going back from a staged agent keeps independent links", () => {
  expect(withoutAgentSwitch({ sources: ["self", "other"] }, "self")).toEqual({
    sources: ["other"],
    next_agent: null,
  });
});
