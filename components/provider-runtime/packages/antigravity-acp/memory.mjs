// Matrix memory for an Antigravity session: the same client, recall and
// learning a Claude or Codex session uses (`@cowboy/memory-client`), with
// its relay requests carried by the worker (`cowboy/memory`) because the
// account unit reaches no host loopback service. The relay's address and
// credential stay with the worker; the token here is a placeholder the
// worker replaces.
import { join } from "node:path";
import { callGuidance as renderCallGuidance, MatrixClient } from "@cowboy/memory-client";

const BRIDGE = "http://cowboy-memory.invalid/";

/** A `fetch` for the memory client that travels over the worker. */
export function bridgedFetch(connection) {
  return async (target, init = {}) => {
    const { pathname } = new URL(target);
    // The relay credential is the worker's; only the request's own
    // headers travel.
    const headers = {};
    for (const [name, value] of Object.entries(init.headers ?? {})) {
      if (name.toLowerCase() !== "authorization") headers[name.toLowerCase()] = String(value);
    }
    let reply;
    try {
      reply = await connection.call("cowboy/memory", {
        path: pathname,
        method: init.method ?? "POST",
        headers,
        body: typeof init.body === "string" ? init.body : "",
      });
    } catch (error) {
      throw new Error(`memory bridge failed: ${error.message}`);
    }
    return new Response(reply?.body ?? "", {
      status: Number.isInteger(reply?.status) ? reply.status : 502,
      headers: reply?.headers ?? {},
    });
  };
}

/**
 * The session's memory client, or `null` when the worker bridges none. Its
 * delivery outbox is private to this Cowboy session (`summary.session`), so
 * evidence one session left behind is only ever delivered by that session,
 * through its own relay.
 */
export async function openMemory(connection, summary, { home, cwd }) {
  const memory = summary?.memory;
  const session = summary?.session;
  if (!memory || !(memory.enable_recall || memory.observe)) return null;
  if (typeof session !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(session)) return null;
  return await MatrixClient.open(
    {
      provider: "antigravity",
      relay: true,
      endpoint: BRIDGE,
      token: "bridged",
      state_dir: join(home, ".cache", "cowboy-memory", session),
      projects: [],
      server: "matrix",
      // Memory's tools are the worker's; this only names them in recall.
      tools: memory.tools === true ? ["memory_put"] : [],
      enableTools: memory.tools === true,
      enableRecall: memory.enable_recall === true,
      observe: memory.observe === true,
      fetch: bridgedFetch(connection),
    },
    connection.descriptor,
    { cwd },
  );
}

/** The installed call guidance, with this session's peer filled in. */
export function callGuidance(summary) {
  return renderCallGuidance(summary ?? null, {
    peer: "Codex",
    peerCommand: "codex",
    fallback: "",
  });
}

/** The text of an ACP prompt (its text blocks). */
export function promptText(prompt) {
  return (Array.isArray(prompt) ? prompt : [])
    .filter((block) => block?.type === "text" && typeof block.text === "string")
    .map((block) => block.text)
    .join("\n");
}

/**
 * What one `session/update` adds to the turn's evidence: the agent's
 * words, and each finished tool call's title and output.
 */
export function updateObservation(update) {
  if (update?.sessionUpdate === "agent_message_chunk" && update.content?.type === "text") {
    return ["assistant", update.content.text];
  }
  if (
    ["tool_call", "tool_call_update"].includes(update?.sessionUpdate) &&
    ["completed", "failed"].includes(update.status)
  ) {
    let output = (Array.isArray(update.content) ? update.content : [])
      .map((item) => (item?.content?.type === "text" ? item.content.text : ""))
      .filter(Boolean)
      .join("\n");
    // A result carried only in `rawOutput` still counts (bounded).
    if (!output && update.rawOutput !== undefined) {
      output = (
        typeof update.rawOutput === "string" ? update.rawOutput : JSON.stringify(update.rawOutput)
      ).slice(0, 20000);
    }
    return ["tool", JSON.stringify({ title: update.title, status: update.status, output })];
  }
  return null;
}
