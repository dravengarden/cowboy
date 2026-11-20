// Matrix memory for an Antigravity session: the same client, recall and
// learning a Claude or Codex session uses (`@cowboy/memory-client`), with
// its relay requests carried by the worker (`cowboy/memory`) because the
// account unit reaches no host loopback service. The relay's address and
// credential stay with the worker; the token here is a placeholder the
// worker replaces.
import { MatrixClient } from "@cowboy/memory-client";

const BRIDGE = "http://cowboy-memory.invalid/";

/** A `fetch` for the memory client that travels over the worker. */
export function bridgedFetch(connection) {
  return async (target, init = {}) => {
    const { pathname } = new URL(target);
    const headers = {};
    for (const [name, value] of Object.entries(init.headers ?? {})) {
      headers[name.toLowerCase()] = String(value);
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
 * The session's memory client, or `null` when the worker bridges none.
 * `stateDir` is a unit-private directory for the delivery outbox.
 */
export async function openMemory(connection, summary, { stateDir, cwd }) {
  const memory = summary?.memory;
  if (!memory || !(memory.enable_recall || memory.observe || memory.enable_tools)) return null;
  const tools = Array.isArray(memory.tools)
    ? memory.tools.filter((tool) => /^[a-z][a-z0-9_]{0,63}$/.test(tool))
    : [];
  return await MatrixClient.open(
    {
      provider: "antigravity",
      relay: true,
      endpoint: BRIDGE,
      token: "bridged",
      state_dir: stateDir,
      projects: [],
      server: typeof memory.server === "string" ? memory.server : "matrix",
      tools,
      enableTools: memory.enable_tools === true && tools.length > 0,
      enableRecall: memory.enable_recall === true,
      observe: memory.observe === true,
      fetch: bridgedFetch(connection),
    },
    connection.descriptor,
    { cwd },
  );
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
    const output = (Array.isArray(update.content) ? update.content : [])
      .map((item) => (item?.content?.type === "text" ? item.content.text : ""))
      .filter(Boolean)
      .join("\n");
    return ["tool", JSON.stringify({ title: update.title, status: update.status, output })];
  }
  return null;
}
