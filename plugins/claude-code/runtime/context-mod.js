// Private Claude Code Mod. Loaded only for Cowboy's bound execution sessions.
// The pinned native CLI supplies this documented API; no native code is patched.
let context;
const unavailable = "Execution context unavailable. Stop and report the error.";
// Native agent id -> its runtime-local output file, for exact projection.
const agentOutputs = new Map();
const agentRegistrations = new Map();
// Stopped agents whose held target calls must end. next.signal reaches a held
// call only once native can process the stop; this also covers a missed abort.
const stoppedAgents = new Set();
const agentTypes = ["general-purpose", "claude", "Explore", "Plan"];

// The native output file is a runtime-home JSONL transcript. Replace only
// the exact registered locator in the launch result and its notification.
export function targetAgentResult(result, outputFile, handle) {
  return {
    ...result,
    result: { ...result.result, outputFile: handle },
    ...(typeof result.text === "string"
      ? { text: result.text.split(outputFile).join(handle) }
      : {}),
  };
}

export function targetTaskNotificationText(text, outputs) {
  return text.replace(
    /<task-notification>[\s\S]*?<\/task-notification>/g,
    (notification) => {
      const id = /<task-id>([^<]*)<\/task-id>/.exec(notification)?.[1];
      const file = outputs.get(id);
      return file === undefined ? notification : notification.replace(
        `<output-file>${file}</output-file>`,
        `<output-file>cowboy-agent://${id}</output-file>`,
      );
    },
  );
}

// Idle notifications append as prompts; one arriving during a turn is a
// queued_command attachment ("delivery" door). Both carry the same element.
export function targetTaskNotification(event, outputs) {
  if (event.origin?.kind !== "task-notification") return event;
  const project = (text) => targetTaskNotificationText(text, outputs);
  return {
    ...event,
    message: {
      ...event.message,
      content: typeof event.message.content === "string"
        ? project(event.message.content)
        : event.message.content.map((block) =>
          block.type === "text"
            ? { ...block, text: project(block.text) }
            : block
        ),
    },
  };
}

function bridgePost($, path, value) {
  return $.http.fetch("http://cowboy-execution" + path, {
    socketPath: context.socketPath,
    method: "POST",
    headers: { Authorization: "Bearer " + context.bridgeToken },
    body: JSON.stringify(value),
  });
}

// Native TaskStop owns the agent. Mark it and cancel every target call it
// still holds, independently of whether each held hook observes its abort.
async function stopAgent($, event, next, agentId) {
  const result = await next(event);
  if (result?.isError !== true && result?.deny === undefined) {
    stoppedAgents.add(agentId);
    await bridgePost($, "/agent-stop", { agentId }).catch(() => {});
  }
  return result;
}

async function agent($, event, next) {
  if (event.agentId !== undefined) {
    return {
      deny:
        "Nested agents are not available in this execution environment; do the work directly.",
    };
  }
  if (event.isolation !== undefined) {
    return {
      deny:
        "Worktree and remote agent isolation would run outside the bound workspace; omit isolation.",
    };
  }
  if (
    event.subagent_type !== undefined &&
    !agentTypes.includes(event.subagent_type)
  ) {
    return {
      deny: `Use one of these agent types: ${agentTypes.join(", ")}.`,
    };
  }
  if (event.run_in_background === false) {
    return {
      deny:
        "Run the agent in the background; its completion notification delivers the result.",
    };
  }
  const result = await next(event);
  const task = result?.result;
  if (
    !task?.isAsync || typeof task.agentId !== "string" ||
    typeof task.outputFile !== "string"
  ) return result;
  const handle = "cowboy-agent://" + task.agentId;
  // Project the notification even if durable registration fails below.
  agentOutputs.set(task.agentId, task.outputFile);
  const projected = targetAgentResult(result, task.outputFile, handle);
  // A fast child can finish before this returns; completion awaits it.
  const registered = bridgePost($, "/agent", {
    agentId: task.agentId,
    toolUseId: event.tool_use_id,
    owner: event.agentId ?? null,
    outputFile: task.outputFile,
  }).then((response) => response.ok, () => false);
  agentRegistrations.set(task.agentId, registered);
  if (await registered) return projected;
  return {
    ...projected,
    text: (projected.text ?? "") +
      "\nIts final answer could not be recorded for a later Read; use its completion notification.",
  };
}

function environment() {
  return { text: context?.environment ?? unavailable };
}

function instructions(_$, event) {
  return {
    blocks: [
      ...event.blocks.filter((block) => block.name === "currentDate"),
      {
        name: "claudeMd",
        text: context?.instructions ?? unavailable,
      },
    ],
    instructionFiles: [],
  };
}

export function targetImageResult(event) {
  if (
    event.origin?.kind !== "tool" ||
    !["Read", "ReadFile", "mcp__cowboy_execution__read"].includes(
      event.origin.tool,
    )
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
  // Compaction restores native Read/Edit paths directly, outside tool.call.
  // Those snapshots describe the runtime filesystem, never the target. Drop
  // their model-facing attachment on every render, including a cold resume;
  // the native summary stays and explicit Read can reacquire target content.
  on("prompt.attachment", { type: "file" }, () => ({ text: null })).catch(
    () => ({ text: null }),
  );
  on("tool.describe", async ($, event, next) => {
    const description = context?.descriptions[event.tool];
    return description ? { description } : next(event);
  }).catch(() => ({ description: unavailable }));

  on("tool.call", async ($, event, next) => {
    if (["TodoWrite", "AskUserQuestion"].includes(event.tool)) {
      return next(event);
    }
    if (
      context?.memory &&
      [
        "memory_search",
        "memory_get",
        "memory_put",
        "memory_forget",
        "memory_read",
        "memory_execute",
        "memory_receipt",
      ].some((
        name,
      ) => event.tool === "mcp__matrix__" + name)
    ) return next(event);
    if (!context) return { deny: unavailable };
    // Native agents keep their native lifecycle; their own tool calls arrive
    // here with agentId and use the same target routing below.
    if (event.tool === "Agent") return agent($, event, next);
    const task = event.task_id ?? event.shell_id;
    if (event.tool === "TaskStop" && agentOutputs.has(task)) {
      return stopAgent($, event, next, task);
    }
    if (event.tool === "SendMessage") {
      if (!agentOutputs.has(event.to) || event.notify_when_idle === true) {
        return {
          deny:
            "SendMessage can only continue this session's background agents by agentId.",
        };
      }
      // The continued round can complete quickly; its completion waits for
      // this durable running state so an older outcome is never restored.
      const resumed = Promise.withResolvers();
      const previous = agentRegistrations.get(event.to);
      agentRegistrations.set(event.to, resumed.promise);
      try {
        await previous;
        const result = await next(event);
        if (result?.isError !== true) {
          stoppedAgents.delete(event.to);
          await bridgePost($, "/agent-resume", { agentId: event.to }).catch(
            () => {},
          );
        }
        return result;
      } finally {
        resumed.resolve();
      }
    }
    if (!context.descriptions[event.tool]) return { deny: unavailable };
    const signal = next.signal;
    const abandoned = () => signal?.aborted || stoppedAgents.has(event.agentId);
    if (abandoned()) return { deny: "Tool call was cancelled" };
    const input = { ...event };
    delete input.tool;
    delete input.tool_use_id;
    delete input.agentId;
    delete input.consent;
    // An interrupted turn or stopped agent abandons this call. Cancel only
    // the target processes it started; the bridge also refuses late admission.
    let cancelled = false;
    const abandon = () => {
      if (cancelled) return;
      cancelled = true;
      bridgePost($, "/cancel", { id: event.tool_use_id }).catch(() => {});
    };
    signal?.addEventListener("abort", abandon, { once: true });
    try {
      let path = "/tool";
      let body = {
        id: event.tool_use_id,
        tool: event.tool,
        input,
        ...(event.agentId === undefined ? {} : { owner: event.agentId }),
      };
      for (;;) {
        const response = await bridgePost($, path, body);
        if (abandoned()) {
          abandon();
          return { deny: "Tool call was cancelled" };
        }
        if (!response.ok) {
          return {
            deny:
              "Execution result unavailable. Inspect state before repeating a mutation.",
          };
        }
        const result = JSON.parse(response.text);
        if (response.status !== 202) return result;
        if (result.pending !== event.tool_use_id) {
          throw new Error("Execution identity changed");
        }
        path = "/result";
        body = { id: event.tool_use_id };
      }
    } finally {
      signal?.removeEventListener("abort", abandon);
    }
  }).catch(() => ({
    deny:
      "Execution interception failed. Local execution is disabled; inspect target state before retrying.",
  }));

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
  // Native completion notifications enter as prompts. Projecting before the
  // append keeps the runtime locator out of history, resume and compaction.
  on(
    "session.append",
    { door: "prompt" },
    (_$, event, next) => next(targetTaskNotification(event, agentOutputs)),
  );
  on(
    "session.append",
    { door: "delivery" },
    (_$, event, next) => next(targetTaskNotification(event, agentOutputs)),
  );
  // The model reads queued notifications through this render, including
  // re-renders after resume; the stored row above is projected separately.
  on(
    "prompt.attachment",
    { type: "queued_command" },
    (_$, event, next) =>
      next({
        ...event,
        text: typeof event.text === "string"
          ? targetTaskNotificationText(event.text, agentOutputs)
          : event.text,
      }),
  );
  on("turn.complete", async ($, event, next) => {
    if (event.agentId !== undefined && agentOutputs.has(event.agentId)) {
      if (event.isAborted === true) stoppedAgents.add(event.agentId);
      try {
        await agentRegistrations.get(event.agentId);
        await bridgePost($, "/agent-complete", {
          agentId: event.agentId,
          answer: event.answer ?? "",
          reason: event.reason ?? "unknown",
          isAborted: event.isAborted === true,
        });
      } catch {
        // A later handle Read reports the agent as unfinished, never invented.
      }
    }
    return next(event);
  });

  on("session.start", async ($, event, next) => {
    const path = await $.env.get("COWBOY_CLAUDE_CONTEXT");
    const loaded = JSON.parse(await $.fs.read(path));
    if (
      loaded.schema !== 1 || !/^[a-f0-9]{32}$/.test(loaded.nonce) ||
      !/^\/tmp\/cowboy-claude-mod-[^/]+\/bridge\.sock$/.test(
        loaded.socketPath,
      ) ||
      !/^[a-f0-9]{64}$/.test(loaded.bridgeToken) ||
      !loaded.descriptions || typeof loaded.descriptions !== "object" ||
      !loaded.agents || typeof loaded.agents !== "object" ||
      ![loaded.environment, loaded.instructions, loaded.git].every((value) =>
        typeof value === "string" && value.length <= 262144
      )
    ) throw new Error("Invalid bound execution context");
    context = Object.freeze(loaded);
    // A resumed session may continue an agent registered by an earlier process.
    for (const [id, file] of Object.entries(loaded.agents)) {
      agentOutputs.set(id, file);
    }
    const response = await $.http.fetch("http://cowboy-execution/ready", {
      socketPath: context.socketPath,
      method: "POST",
      headers: { Authorization: "Bearer " + context.bridgeToken },
      body: "{}",
    });
    if (!response.ok || JSON.parse(response.text).ready !== true) {
      throw new Error("Execution bridge unavailable");
    }
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
