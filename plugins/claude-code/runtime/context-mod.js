// Private Claude Code Mod. Loaded only for Cowboy's bound execution sessions.
// The pinned native CLI supplies this documented API; no native code is patched.
let context;
const unavailable = "Execution context unavailable. Stop and report the error.";

function environment() {
  return { text: context?.environment ?? unavailable };
}

function instructions(_$, event) {
  return {
    blocks: [
      ...event.blocks.filter((block) => block.name === "currentDate"),
      { name: "claudeMd", text: context?.instructions ?? unavailable },
    ],
    instructionFiles: [],
  };
}

export function targetImageResult(event) {
  if (
    event.origin?.kind !== "tool" ||
    !["ReadFile", "mcp__cowboy_execution__read"].includes(event.origin.tool)
  ) return event;
  return {
    ...event,
    message: {
      ...event.message,
      content: event.message.content.map((block) => {
        if (
          block.type !== "tool_result" || !Array.isArray(block.content) ||
          !block.content.some((item) => item.type === "image")
        ) return block;
        // Native MCP image persistence appends an OVH cache path. The facade
        // already supplies the real target path. Keep the original media and
        // target caption, removing only this generated native locator before
        // it enters the durable transcript (including resume/compaction).
        return {
          ...block,
          content: block.content.filter((item) =>
            item.type !== "text" ||
            !/^\[Image: source: .*\/tool-results\/mcp-cowboy_execution-blob-[^\n]+\]$/
              .test(item.text)
          ),
        };
      }),
    },
  };
}

export function targetCompactionResult(event) {
  if (event.door !== "compaction" || event.origin?.kind !== "engine") {
    return event;
  }
  return {
    ...event,
    message: {
      ...event.message,
      content: event.message.content.map((block) =>
        block.type === "text"
          ? {
            ...block,
            text: block.text.replace(
              /\nIf you need specific details from before compaction \(like exact code snippets, error messages, or content you generated\), read the full transcript at: [^\n]+\.jsonl\n/g,
              "\n",
            ),
          }
          : block
      ),
    },
  };
}

export function register(on) {
  on("prompt.attachment", { type: "environment" }, environment).catch(
    environment,
  );
  on("prompt.attachment", { type: "session_context" }, () => ({
    text: context?.git ?? unavailable,
  })).catch(() => ({ text: unavailable }));
  on("prompt.context", instructions).catch(instructions);
  on("prompt.section", { name: "env_info" }, environment).catch(environment);
  on(
    "session.append",
    { door: "tool-result" },
    (_$, event, next) => next(targetImageResult(event)),
  );
  on(
    "session.append",
    { door: "compaction" },
    (_$, event, next) => next(targetCompactionResult(event)),
  );

  on("session.start", async ($, event, next) => {
    const path = await $.env.get("COWBOY_CLAUDE_CONTEXT");
    const loaded = JSON.parse(await $.fs.read(path));
    if (
      loaded.schema !== 1 || !/^[a-f0-9]{32}$/.test(loaded.nonce) ||
      ![loaded.environment, loaded.instructions, loaded.git].every((value) =>
        typeof value === "string" && value.length <= 262144
      )
    ) throw new Error("Invalid bound execution context");
    context = Object.freeze(loaded);
    // This is a readiness receipt, never a model tool. Cowboy first issues the
    // native /cost command (which cannot call a model), then asks initialize for
    // this exact per-process name. Unknown slash commands MUST NOT be used as
    // readiness probes: native Claude can submit them to the model.
    await $.command.register({
      name: "cowboy-execution-ready-" + context.nonce,
      description: "Cowboy execution context loaded",
    });
    return next(event);
  });
}
