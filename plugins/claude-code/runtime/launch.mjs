import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { once } from "node:events";
import {
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  rm,
  writeFile,
} from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, posix } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  bindingKey,
  DESCRIPTIONS,
  NATIVE_TOOLS,
  WorkspaceTools,
} from "./tools.mjs";
import { startModBridge } from "./mod-bridge.mjs";
import {
  BUNDLED_SKILLS,
  SKILL_PLUGIN,
  targetSkills,
  UNAVAILABLE_BUNDLED,
  writeSkillPlugin,
} from "./skills.mjs";
import {
  claudeObservation,
  claudePrompt,
  localMemoryNative,
  MATRIX_TOOLS,
  MatrixClient,
  matrixConfiguration,
} from "./memory.mjs";

const privateCli = "COWBOY_PRIVATE_CLAUDE_EXECUTABLE";
const forbiddenTools = [
  "EnterWorktree",
  "ExitWorktree",
  "CronCreate",
  "CronDelete",
  "CronList",
  "Computer",
  "ToolSearch",
  "EnterPlanMode",
  "ExitPlanMode",
];

// Native tools that run where the session runs, as in a local session:
// lifecycle, task list, web and review reporting. context-mod.js restricts
// Agent and SendMessage to background agents of this session; their own
// tools still route to target. WebFetch refuses the target's loopback names.
// Skill expands the target's skills (skills.mjs) and allowed bundled ones.
export const NATIVE_PASSTHROUGH = [
  "AskUserQuestion",
  "Agent",
  "SendMessage",
  "Skill",
  "TaskCreate",
  "TaskGet",
  "TaskList",
  "TaskUpdate",
  "WebFetch",
  "WebSearch",
  "ReportFindings",
];

// Native's default tool set (2.1.287) has no Glob or Grep: searches use Bash.
const ADVERTISED_TOOLS = NATIVE_TOOLS.filter((tool) =>
  !["Glob", "Grep"].includes(tool)
);

// Client completion frames name the native runtime-home output file. Replace
// only the exact locator registered for that agent with its handle.
export function targetTaskFrame(frame, agents) {
  const agent = frame.type === "system" && typeof frame.task_id === "string" &&
      Object.hasOwn(agents, frame.task_id)
    ? agents[frame.task_id]
    : undefined;
  return agent && frame.output_file === agent.outputFile
    ? { ...frame, output_file: "cowboy-agent://" + frame.task_id }
    : frame;
}

// The permission mode the native process starts with: the last forwarded
// value, else native's own default. Cowboy selects one per session.
export function startingPermissionMode(argv) {
  const index = argv.lastIndexOf("--permission-mode");
  return index < 0 ? "default" : argv[index + 1];
}

// The native shape of an approval request (SDKControlPermissionRequest). The
// facade asks only after native rules and mode answered "ask". Suggestions
// stay empty: a persistent rule could not reach native's own rule store.
export function permissionRequest({ id, tool, input, reason, owner }) {
  const description = tool === "Bash"
    ? input.command
    : input.file_path ?? input.notebook_path ?? input.path ?? input.pattern;
  return {
    subtype: "can_use_tool",
    tool_name: tool,
    display_name: tool,
    input,
    permission_suggestions: [],
    tool_use_id: id,
    ...(owner === undefined ? {} : { agent_id: owner }),
    ...(reason ? { decision_reason: reason } : {}),
    ...(typeof description === "string" ? { description } : {}),
  };
}

export function permissionResult(response) {
  const result = response?.subtype === "success" ? response.response : null;
  if (result?.behavior === "allow" || result?.behavior === "deny") {
    return result;
  }
  return {
    behavior: "deny",
    message: typeof response?.error === "string"
      ? response.error
      : "Approval failed; the tool did not run",
  };
}

// Host approvals for target tools. The host answers on the same stdio channel
// as native's own requests, so responses are matched by private request ids.
export class PermissionBroker {
  constructor(mode) {
    this.mode = mode;
    this.pending = new Map();
  }

  request(call) {
    // dontAsk: native denies whatever would otherwise prompt.
    if (this.mode === "dontAsk" || !this.send) {
      return Promise.resolve({
        behavior: "deny",
        message:
          `Permission to use ${call.tool} was denied: this session does not ask for approval.`,
      });
    }
    const requestId = "cowboy-permission-" + randomUUID();
    return new Promise((resolve) => {
      this.pending.set(requestId, { id: call.id, resolve });
      this.send({
        type: "control_request",
        request_id: requestId,
        request: permissionRequest(call),
      }).catch(() =>
        this.settle(requestId, {
          behavior: "deny",
          message: "Approval could not be requested; the tool did not run",
        })
      );
    });
  }

  respond(frame) {
    const requestId = frame.response?.request_id;
    if (frame.type !== "control_response" || !this.pending.has(requestId)) {
      return false;
    }
    this.settle(requestId, permissionResult(frame.response));
    return true;
  }

  cancel(id) {
    for (const [requestId, entry] of this.pending) {
      if (entry.id !== id) continue;
      this.send({ type: "control_cancel_request", request_id: requestId })
        .catch(() => {});
      this.settle(requestId, {
        behavior: "deny",
        message: "Tool call was cancelled",
      });
    }
  }

  settle(requestId, result) {
    const entry = this.pending.get(requestId);
    if (!entry) return;
    this.pending.delete(requestId);
    this.onResult?.(result);
    entry.resolve(result);
  }
}

// Target project hooks for native Claude, as the project wrote them: native
// keeps matching, ordering, timeouts, output parsing and its own messages.
// Their command hooks run on the target through the shell prefix. Tool hooks
// for target tools also reach the Mod adapter, because a tool the facade
// answers never reaches native's own PreToolUse/PostToolUse.
export function shellQuote(value) {
  return "'" + value.replaceAll("'", "'\\''") + "'";
}

export function projectHookSettings(hooks) {
  const commands = [];
  const tool = {};
  const settings = {};
  for (const [event, groups] of Object.entries(hooks)) {
    settings[event] = groups.filter((group) =>
      group && typeof group === "object" && Array.isArray(group.hooks)
    ).map((group) => ({
      ...group,
      hooks: group.hooks.map((hook) => {
        // Native never fires these for target tools the facade answers.
        const toolEvent = [
          "PreToolUse",
          "PostToolUse",
          "PostToolUseFailure",
          "PermissionRequest",
        ].includes(event);
        const command = hook?.type === "command" &&
          typeof hook.command === "string";
        // The adapter cannot reproduce these around a target tool; it refuses
        // a matching call instead of skipping the project's review.
        const unsupported = !command
          ? `${String(hook?.type ?? "unknown")} hook`
          : hook.if !== undefined
          ? "hook with an if condition"
          : hook.asyncRewake === true
          ? "asyncRewake hook"
          : undefined;
        if (toolEvent && unsupported) {
          (tool[event] ??= []).push({ matcher: group.matcher, unsupported });
        }
        if (!command) return hook;
        // Exec form bypasses the shell prefix natively; hand native an
        // equivalent shell-form string the prefix routes, run as argv.
        const argv = Array.isArray(hook.args) &&
            hook.args.every((value) => typeof value === "string")
          ? [hook.command, ...hook.args]
          : undefined;
        const shellCommand = argv
          ? argv.map(shellQuote).join(" ")
          : hook.command;
        // Native enforces each hook's own timeout by ending the proxy; this
        // is the target's backstop (natively, 600 s; async hooks unbounded).
        const timeout = Number.isSafeInteger(hook.timeout) &&
            hook.timeout > 0 && hook.timeout <= 3600
          ? hook.timeout
          : hook.async === true
          ? 3600
          : 600;
        const index = commands.push({
          command: shellCommand,
          timeout,
          ...(argv ? { argv } : {}),
          ...(hook.async === true ? { async: true } : {}),
        }) - 1;
        if (toolEvent && !unsupported) {
          (tool[event] ??= []).push({ matcher: group.matcher, index });
        }
        if (!argv) return hook;
        const { args: _args, ...shellForm } = hook;
        return { ...shellForm, command: shellCommand };
      }),
    }));
  }
  return { settings: { hooks: settings }, commands, tool };
}

export function nativeArguments(
  args,
  plugin,
  memoryConfig,
  hookSettings,
  skillPlugin,
) {
  const values = new Set([
    "--model",
    "--fallback-model",
    "--effort",
    "--resume",
    "--resume-session-at",
    "--session-id",
    "--max-turns",
    "--max-budget-usd",
    "--max-thinking-tokens",
    "--thinking-display",
    "--betas",
    "--thinking",
    "--task-budget",
    "--json-schema",
    "--permission-mode",
    "--permission-prompts",
    "--permission-prompt-tool",
  ]);
  // The ACP adapter supplies some of these itself. In a bound session Cowboy
  // owns the tool/context surfaces instead of that adapter's local file tools.
  const replaced = new Set([
    "--output-format",
    "--input-format",
    "--tools",
    "--disallowedTools",
    "--allowedTools",
    "--setting-sources",
    "--settings",
    "--mcp-config",
    "--append-system-prompt",
    "--system-prompt",
    "--plugin-dir",
    "--agents",
    "--add-dir",
    "--project-config-root",
  ]);
  const flags = new Set([
    "--print",
    "-p",
    "--verbose",
    "--include-partial-messages",
    "--replay-user-messages",
    "--allow-dangerously-skip-permissions",
    "--dangerously-skip-permissions",
    "--strict-mcp-config",
    "--disable-slash-commands",
    "--no-session-persistence",
    "--include-hook-events",
    "--fork-session",
  ]);
  const forwarded = [];
  const disallowed = new Set(forbiddenTools);
  // The pinned SDK uses both --name value and --name=value, including an
  // intentionally empty --setting-sources= value.
  args = args.flatMap((argument) => {
    const equals = argument.startsWith("--") ? argument.indexOf("=") : -1;
    return equals < 0
      ? [argument]
      : [argument.slice(0, equals), argument.slice(equals + 1)];
  });
  for (let index = 0; index < args.length; index++) {
    const argument = args[index];
    if (values.has(argument) || replaced.has(argument)) {
      const value = args[++index];
      if (typeof value !== "string" || value.startsWith("--")) {
        throw new Error("Invalid native option");
      }
      if (argument === "--permission-mode" && value === "plan") {
        throw new Error(
          "Native plan files are unavailable in an execution session",
        );
      }
      if (argument === "--permission-prompt-tool" && value !== "stdio") {
        throw new Error("Execution permissions require the native SDK channel");
      }
      if (argument === "--disallowedTools") {
        for (const tool of value.split(",")) {
          disallowed.add(tool);
        }
      }
      if (values.has(argument)) forwarded.push(argument, value);
    } else if (flags.has(argument)) {
      // The pinned ACP adapter encodes replay-user-messages as an empty-valued
      // extraArgs entry; the SDK emits a trailing empty argv element for it.
      if (args[index + 1] === "") index++;
      if (
        [
          "--include-partial-messages",
          "--replay-user-messages",
          "--include-hook-events",
          "--fork-session",
          "--allow-dangerously-skip-permissions",
          "--dangerously-skip-permissions",
        ].includes(
          argument,
        )
      ) forwarded.push(argument);
    } else {
      const option = /^--[a-zA-Z0-9-]+$/.test(argument)
        ? argument
        : "unsupported argument";
      const error = new Error(
        `Native option is unavailable in an execution session: ${option}`,
      );
      error.cowboyDiagnostic = error.message;
      throw error;
    }
  }
  return [
    "--print",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--tools",
    [...ADVERTISED_TOOLS, ...NATIVE_PASSTHROUGH].join(","),
    "--disallowedTools",
    [...disallowed].join(","),
    "--setting-sources",
    "",
    "--strict-mcp-config",
    "--plugin-dir",
    plugin,
    ...(skillPlugin ? ["--plugin-dir", skillPlugin] : []),
    ...(memoryConfig
      ? ["--mcp-config", memoryConfig, "--allowedTools", MATRIX_TOOLS.join(",")]
      : []),
    ...(hookSettings ? ["--settings", hookSettings] : []),
    ...forwarded,
  ];
}

async function* frames(stream) {
  let buffer = "";
  stream.setEncoding("utf8");
  for await (const chunk of stream) {
    buffer += chunk;
    if (Buffer.byteLength(buffer) > 16 * 1024 * 1024) {
      throw new Error("Native frame exceeds limit");
    }
    let newline;
    while ((newline = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, newline);
      buffer = buffer.slice(newline + 1);
      if (line.trim()) yield JSON.parse(line);
    }
  }
  if (buffer.trim()) throw new Error("Incomplete native frame");
}

async function send(stream, frame) {
  if (!stream.write(JSON.stringify(frame) + "\n")) await once(stream, "drain");
}

// Local commands an execution session runs natively; skills.mjs adds the
// target's skills and commands and the bundled skills that work here.
const LOCAL_COMMANDS = ["compact", "cost", "context", "status", "help"];

// The name a native command is shown and typed by in this session, or
// undefined where the session refuses it.
export function exposedCommand(name, skills) {
  if (name.startsWith(skills.prefix)) {
    const target = name.slice(skills.prefix.length);
    return skills.entries.some((entry) => entry.name === target)
      ? target
      : undefined;
  }
  // A target skill shadows the bundled one of its name, even one that
  // cannot run here.
  if (
    [...skills.entries, ...skills.omitted].some((entry) => entry.name === name)
  ) return undefined;
  if (
    !/^[a-z][a-z0-9-]*$/i.test(name) || LOCAL_COMMANDS.includes(name) ||
    skills.bundled.includes(name)
  ) return name;
  return undefined;
}

// Native's skills allowlist: the target's and the bundled skills that work.
export function skillAllowlist(skills) {
  return [
    ...skills.entries.map((entry) => skills.prefix + entry.name),
    ...skills.bundled.filter((name) =>
      ![...skills.entries, ...skills.omitted].some((entry) =>
        entry.name === name
      )
    ),
  ];
}

// A user's slash command, as native names it here; refused if unavailable.
// Other text, a path like /home/u/file included, is an ordinary prompt.
export function nativeCommand(prompt, skills) {
  const typed = /^\/(\S+)/.exec(prompt.trim())?.[1];
  if (typed === undefined) return undefined;
  if (skills.entries.some((entry) => entry.name === typed)) {
    return skills.prefix + typed;
  }
  if (!/^[a-z][a-z0-9-]*$/i.test(typed)) return undefined;
  if (exposedCommand(typed, skills) === undefined) {
    throw new Error("This command is unavailable in an execution session");
  }
  return typed;
}

export function initializeRequest(frame, allowlist = []) {
  return {
    ...frame,
    request: {
      subtype: "initialize",
      sdkMcpServers: [],
      toolAliases: {},
      excludeDynamicSections: true,
      skills: allowlist,
      ...(Array.isArray(frame.request.supportedDialogKinds)
        ? { supportedDialogKinds: frame.request.supportedDialogKinds }
        : {}),
    },
  };
}

export function allowedControl(request) {
  if (request.subtype === "apply_flag_settings") {
    const settings = request.settings;
    return settings && typeof settings === "object" &&
      !Array.isArray(settings) &&
      Object.entries(settings).every(([name, value]) =>
        (name === "effortLevel" &&
          (value === null ||
            (typeof value === "string" && value.length <= 24))) ||
        (name === "fastMode" && typeof value === "boolean")
      );
  }
  return new Set([
    "interrupt",
    "set_permission_mode",
    "set_model",
    "set_max_thinking_tokens",
    "rename_session",
    "set_color",
    "mcp_status",
    "get_context_usage",
    "get_session_cost",
    "list_models",
    "get_usage",
    "get_binary_version",
    "get_settings",
    "get_hooks_listing",
    "list_permission_rules",
    "cancel_async_message",
  ]).has(request.subtype);
}

async function bridge(child, tools, context, memory, broker) {
  let resolveReady, rejectReady;
  const ready = new Promise((resolve, reject) => {
    resolveReady = resolve;
    rejectReady = reject;
  });
  ready.catch(() => {});
  let initial;
  let initialReply;
  const pendingInterrupts = new Map();
  // Requests this launcher sends native itself; their replies stay private.
  const internal = new Map();
  const modeRequests = new Map();
  let stage = "initialize";
  // Native aborts the turn and background agents before it answers an
  // interrupt. Cancelling target processes first lets a held child call
  // return their exit status to a still-running agent, which then continues.
  const interruptCancel = () => {
    let cancelling;
    const cancel = () =>
      cancelling ??= tools.cancelForeground().then(
        (pending) =>
          pending.length
            ? "Target cancellation is pending; retained task handles: " +
              pending.map((id) => `cowboy-task://${id}`).join(", ")
            : undefined,
        () =>
          "Target cancellation could not be saved; inspect target tasks before retrying commands",
      );
    // An unresponsive native process must not keep target work alive.
    setTimeout(cancel, 5000).unref();
    return cancel;
  };
  broker.send = (frame) => send(process.stdout, frame);
  broker.onResult = (result) => {
    const mode = result.behavior === "allow"
      ? result.updatedPermissions?.find((update) =>
        update.type === "setMode" && update.destination === "session" &&
        update.mode !== "plan"
      )?.mode
      : undefined;
    const request = mode
      ? { subtype: "set_permission_mode", mode }
      : result.behavior === "deny" && result.interrupt === true
      ? { subtype: "interrupt" }
      : undefined;
    if (!request) return;
    const id = "cowboy-internal-" + randomUUID();
    if (mode) internal.set(id, () => broker.mode = mode);
    else {
      const cancel = interruptCancel();
      internal.set(id, () => cancel());
    }
    send(child.stdin, { type: "control_request", request_id: id, request })
      .catch(() => {});
  };
  const checkId = randomUUID();
  const privateCommand = "cowboy-execution-ready-" + context.nonce;
  const timeout = setTimeout(
    () => rejectReady(new Error("Claude execution module did not initialize")),
    30000,
  );
  // The client sees commands by the names it can type here: the target's
  // skills under their own names, without commands this session refuses.
  const cleanCommands = (frame) => {
    const result = structuredClone(frame);
    const exposed = (name) =>
      typeof name === "string" &&
        !name.startsWith("cowboy-execution-ready-")
        ? exposedCommand(name, context.skills)
        : undefined;
    for (const value of [result, result.response?.response]) {
      if (Array.isArray(value?.commands)) {
        value.commands = value.commands.flatMap((command) => {
          const name = exposed(command?.name);
          return name === undefined ? [] : [{ ...command, name }];
        });
      }
    }
    if (result.type === "system" && result.subtype === "init") {
      for (const key of ["slash_commands", "skills"]) {
        if (Array.isArray(result[key])) {
          result[key] = result[key].flatMap((name) => {
            const shown = exposed(name);
            return shown === undefined ? [] : [shown];
          });
        }
      }
    }
    return result;
  };
  const input = (async () => {
    for await (const frame of frames(process.stdin)) {
      if (
        frame.type === "control_request" &&
        frame.request.subtype === "initialize"
      ) {
        if (initial) throw new Error("Duplicate native initialization");
        initial = initializeRequest(frame, skillAllowlist(context.skills));
        await send(child.stdin, initial);
        continue;
      }
      await ready;
      if (broker.respond(frame)) continue;
      if (frame.type === "control_request") {
        const subtype = frame.request.subtype;
        if (
          subtype === "set_permission_mode" && frame.request.mode === "plan"
        ) {
          await send(process.stdout, {
            type: "control_response",
            response: {
              subtype: "error",
              request_id: frame.request_id,
              error:
                "Native plan files are not supported in an execution session",
            },
          });
          continue;
        }
        if (!allowedControl(frame.request)) {
          await send(process.stdout, {
            type: "control_response",
            response: {
              subtype: "error",
              request_id: frame.request_id,
              error: "Execution configuration is owned by Cowboy",
            },
          });
          continue;
        }
        if (subtype === "interrupt") {
          pendingInterrupts.set(frame.request_id, interruptCancel());
        }
        if (subtype === "set_permission_mode") {
          modeRequests.set(frame.request_id, frame.request.mode);
        }
      }
      // These local commands can change native tools/settings or launch local
      // helpers. Ordinary text, /compact and harmless local status remain native.
      if (frame.type === "user") {
        const content = frame.message?.content;
        const prompt = typeof content === "string"
          ? content
          : Array.isArray(content)
          ? content.filter((item) => item.type === "text").map((item) =>
            item.text
          ).join("\n")
          : "";
        const command = nativeCommand(prompt, context.skills);
        // A target skill's command reaches native under its plugin name.
        const typed = /^\s*\/(\S+)/.exec(prompt)?.[1];
        if (command !== undefined && command !== typed) {
          const rename = (text) =>
            text.replace(/^(\s*\/)\S+/, (_match, slash) => slash + command);
          const content = frame.message.content;
          if (typeof content === "string") {
            frame.message.content = rename(content);
          } else {
            const first = content.findIndex((item) => item.type === "text");
            content[first] = {
              ...content[first],
              text: rename(content[first].text),
            };
          }
        }
        if (memory && prompt.trim() && !command) {
          const recalled = await memory.begin(claudePrompt(frame));
          const content = frame.message.content;
          frame.message.content = [
            { type: "text", text: recalled },
            ...(typeof content === "string"
              ? [{ type: "text", text: content }]
              : content),
          ];
        }
      }
      await send(child.stdin, frame);
    }
    child.stdin.end();
  })();
  const output = (async () => {
    for await (const frame of frames(child.stdout)) {
      if (
        stage === "initialize" && frame.type === "control_response" &&
        frame.response.request_id === initial?.request_id
      ) {
        if (frame.response.subtype !== "success") {
          throw new Error("Claude initialization failed");
        }
        initialReply = frame;
        stage = "cost";
        await send(child.stdin, {
          type: "user",
          message: { role: "user", content: "/cost" },
          parent_tool_use_id: null,
          session_id: "",
        });
        continue;
      }
      if (stage === "cost" && frame.type === "result") {
        // A resumed session can first settle an agent notification queued
        // by its previous process. Only one that made no model request may
        // precede readiness; it never reaches the client.
        if (
          frame.origin?.kind === "task-notification" && !frame.is_error &&
          frame.num_turns === 0 && frame.duration_api_ms === 0
        ) continue;
        if (
          frame.is_error || frame.local_command !== "cost" ||
          frame.duration_api_ms !== 0
        ) throw new Error("Claude readiness command failed");
        stage = "check";
        await send(child.stdin, { ...initial, request_id: checkId });
        continue;
      }
      if (
        stage === "check" && frame.type === "control_response" &&
        frame.response.request_id === checkId
      ) {
        if (
          frame.response.subtype !== "success" ||
          !frame.response.response?.commands?.some((command) =>
            command.name === privateCommand
          )
        ) {
          const error = new Error(
            "Claude execution module is missing or disabled",
          );
          error.cowboyDiagnostic = error.message;
          throw error;
        }
        stage = "ready";
        clearTimeout(timeout);
        await send(process.stdout, cleanCommands(initialReply));
        resolveReady();
        continue;
      }
      if (stage === "ready") {
        const responseId = frame.type === "control_response"
          ? frame.response.request_id
          : undefined;
        if (internal.has(responseId)) {
          const settle = internal.get(responseId);
          internal.delete(responseId);
          if (frame.response.subtype === "success") settle();
          continue;
        }
        if (modeRequests.has(responseId)) {
          if (frame.response.subtype === "success") {
            broker.mode = modeRequests.get(responseId);
          }
          modeRequests.delete(responseId);
        }
        // Each turn's init reports native's current permission mode.
        if (
          frame.type === "system" && frame.subtype === "init" &&
          typeof frame.permissionMode === "string"
        ) broker.mode = frame.permissionMode;
        if (
          frame.type === "control_response" &&
          pendingInterrupts.has(frame.response.request_id)
        ) {
          const cancel = pendingInterrupts.get(frame.response.request_id);
          pendingInterrupts.delete(frame.response.request_id);
          const error = await cancel();
          if (error) {
            await send(process.stdout, {
              type: "control_response",
              response: {
                subtype: "error",
                request_id: frame.response.request_id,
                error,
              },
            });
            continue;
          }
        }
        const observation = claudeObservation(frame);
        if (memory && observation) memory.add(...observation);
        if (memory && frame.type === "result" && !frame.local_command) {
          await memory.finish();
        }
        await send(
          process.stdout,
          cleanCommands(targetTaskFrame(frame, tools.state.agents)),
        );
      } else if (frame.type === "control_request") {
        throw new Error(
          "Unexpected native request during execution initialization",
        );
      }
    }
    throw new Error("Native Claude ended");
  })();
  try {
    await Promise.race([
      input,
      output,
      ready.then(() => new Promise(() => {})),
    ]);
  } finally {
    clearTimeout(timeout);
    rejectReady(new Error("Native Claude ended"));
  }
}

async function native(args) {
  const { Connection, readDescriptor } = await import("./connection.mjs");
  const executable = process.env[privateCli];
  if (!isAbsolute(executable ?? "")) {
    throw new Error("Claude requires its exact Machine-bound executable");
  }
  const descriptorPath = process.env.COWBOY_EXECUTION_DESCRIPTOR;
  if (!descriptorPath) {
    return await localMemoryNative(executable, args);
  }
  const descriptor = await readDescriptor(descriptorPath);
  const root = join(
    dirname(descriptorPath),
    ".cowboy-claude-" + bindingKey(descriptor.binding).slice(0, 24),
  );
  await mkdir(root, { mode: 0o700, recursive: true });
  const stat = await lstat(root);
  if (!stat.isDirectory() || (stat.mode & 0o077)) {
    throw new Error("Invalid private execution directory");
  }
  const stage = await mkdtemp(join(root, "native-"));
  let child, connection, modBridge, memory;
  try {
    connection = await Connection.open(descriptor);
    const tools = new WorkspaceTools(
      connection,
      descriptor.binding,
      join(root, "state.json"),
    );
    await tools.load();
    memory = await MatrixClient.open(
      await matrixConfiguration("claude"),
      descriptor,
    );
    if (memory) {
      const nativeCall = tools.nativeCall.bind(tools);
      tools.nativeCall = async (tool, input, call) => {
        const result = await nativeCall(tool, input, call);
        memory.add(
          "tool",
          JSON.stringify(
            { tool, input, result },
            (key, value) =>
              ["data", "base64"].includes(key) ? undefined : value,
          ),
        );
        return result;
      };
    }
    const context = await tools.context();
    // The exact native argv below sets the starting mode before any tool runs.
    const broker = new PermissionBroker("default");
    modBridge = await startModBridge(tools, {
      permissions: broker,
      transcriptRoot: join(
        process.env.CLAUDE_CONFIG_DIR ?? join(homedir(), ".claude"),
        "projects",
      ),
    });
    context.socketPath = modBridge.socketPath;
    context.bridgeToken = modBridge.token;
    context.descriptions = DESCRIPTIONS;
    context.agents = tools.agentLocators();
    context.hooks = { commands: [], tool: {} };
    const projectHooks = await tools.projectHooks();
    // Native evaluates permission paths against its own working directory.
    context.targetCwd = posix.resolve(descriptor.binding.workspace.cwd);
    context.runtimeCwd = process.cwd();
    context.targetHome = tools.home() ?? null;
    context.memory = Boolean(memory);
    const plugin = join(stage, "plugin");
    await mkdir(join(plugin, ".claude-plugin"), { recursive: true });
    await mkdir(join(plugin, "hooks"));
    await writeFile(
      join(plugin, ".claude-plugin", "plugin.json"),
      JSON.stringify({ name: "cowboy-execution", version: "1.0.0" }),
    );
    await writeFile(
      join(plugin, "hooks", "hooks.json"),
      JSON.stringify({ modules: ["./register.js"] }),
    );
    await copyFile(
      new URL("./context-mod.js", import.meta.url),
      join(plugin, "hooks", "register.js"),
    );
    // The target's skills, as a private plugin native loads (skills.mjs).
    const skills = targetSkills(await tools.skillFiles());
    const skillPlugin = skills.entries.length
      ? join(stage, "target-skills")
      : undefined;
    context.skills = skillPlugin
      ? await writeSkillPlugin(
        skillPlugin,
        skills,
        "cowboy" + randomUUID().replaceAll("-", ""),
        context.targetCwd,
      )
      : {
        prefix: SKILL_PLUGIN + ":",
        entries: [],
        omitted: skills.omitted,
        bundled: BUNDLED_SKILLS,
        unavailable: UNAVAILABLE_BUNDLED,
      };
    const memoryConfig = memory ? join(stage, "matrix-mcp.json") : undefined;
    let hookSettings;
    let hookPrefix;
    if (Object.keys(projectHooks).length) {
      const proxy = join(stage, "hook-proxy.mjs");
      await copyFile(new URL("./hook-proxy.mjs", import.meta.url), proxy);
      const quote = shellQuote;
      // Native passes each hook command as one argument. The proxy runs
      // registered project commands on the target, anything else locally.
      hookPrefix = join(stage, "hook-prefix.sh");
      await writeFile(
        hookPrefix,
        `#!/bin/sh\nif [ "$#" -eq 1 ]; then exec ${quote(process.execPath)} ${
          quote(proxy)
        } "$1"; fi\nexec "$@"\n`,
        { mode: 0o700, flag: "wx" },
      );
      const hooks = projectHookSettings(projectHooks);
      // SessionStart hooks may persist exports for later Bash commands.
      tools.hookEnvironment = true;
      context.hooks = { commands: hooks.commands, tool: hooks.tool };
      hookSettings = join(stage, "project-hooks.json");
      await writeFile(hookSettings, JSON.stringify(hooks.settings), {
        mode: 0o600,
        flag: "wx",
      });
    }
    // Native background tasks that stand for target commands left running
    // run this waiter, so native delivers their completion notifications.
    const taskWait = join(stage, "task-wait.mjs");
    await copyFile(new URL("./task-wait.mjs", import.meta.url), taskWait);
    context.taskWait = `${shellQuote(process.execPath)} ${
      shellQuote(taskWait)
    }`;
    const contextPath = join(stage, "context.json");
    await writeFile(contextPath, JSON.stringify(context), {
      mode: 0o600,
      flag: "wx",
    });
    if (memory) {
      await writeFile(
        memoryConfig,
        JSON.stringify({ mcpServers: { matrix: memory.mcp() } }),
        { mode: 0o600, flag: "wx" },
      );
    }
    const environment = {
      ...process.env,
      COWBOY_CLAUDE_CONTEXT: contextPath,
      ...(hookPrefix ? { CLAUDE_CODE_SHELL_PREFIX: hookPrefix } : {}),
      CLAUDE_CODE_DISABLE_AUTO_MEMORY: "1",
      // Native tool bodies never run for project operations. Keep local
      // checkpoints disabled; context-mod.js drops the attachments that
      // describe this machine's files and projects target context instead.
      DISABLE_TELEMETRY: "1",
      DISABLE_ERROR_REPORTING: "1",
      DISABLE_AUTOUPDATER: "1",
      CLAUDE_CODE_DISABLE_FILE_CHECKPOINTING: "1",
      DISABLE_FILE_CHECKPOINTING: "1",
    };
    // Mods' HTTP API also applies this flag to Unix sockets. Keep individual
    // telemetry/error-reporting/update switches disabled while allowing the
    // private execution socket. Never change the user's persisted settings.
    delete environment.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC;
    delete environment.COWBOY_EXECUTION_DESCRIPTOR;
    delete environment[privateCli];
    const nativeArgv = nativeArguments(
      args,
      plugin,
      memoryConfig,
      hookSettings,
      skillPlugin,
    );
    broker.mode = startingPermissionMode(nativeArgv);
    child = spawn(executable, nativeArgv, {
      env: environment,
      stdio: ["pipe", "pipe", "inherit"],
    });
    for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
      process.on(signal, () => child.kill(signal));
    }
    const exit = new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (code) => resolve(code ?? 1));
    });
    bridge(child, tools, context, memory, broker).catch((error) => {
      process.stderr.write(
        (error.cowboyDiagnostic ??
          "Cowboy Claude execution initialization or transport failed; local fallback is disabled") +
          "\n",
      );
      child.kill("SIGTERM");
    });
    process.exitCode = await exit;
    process.stdin.destroy();
  } finally {
    child?.kill("SIGTERM");
    connection?.close();
    await memory?.close();
    await modBridge?.close();
    await rm(stage, { recursive: true, force: true });
  }
}

export async function main(args) {
  const inspect = args.slice(1);
  if (
    args[0] === "--cowboy-private-cli" &&
    (JSON.stringify(inspect) === '["--version"]' ||
      JSON.stringify(inspect) === '["auth","status","--json"]')
  ) {
    const executable = process.env[privateCli];
    if (!isAbsolute(executable ?? "")) {
      throw new Error("Missing exact Claude executable");
    }
    // The ACP adapter probes account status concurrently with session startup.
    // Native authentication belongs to OVH and must not occupy the one target
    // execution connection (or be mistaken for an Agent tool invocation).
    const environment = { ...process.env };
    delete environment.COWBOY_EXECUTION_DESCRIPTOR;
    delete environment[privateCli];
    const child = spawn(executable, inspect, {
      env: environment,
      stdio: "inherit",
    });
    process.exitCode = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (code) => resolve(code ?? 1));
    });
    return;
  }
  if (args[0] === "--cowboy-private-cli") return await native(args.slice(1));
  const inspection = args.length === 1 &&
    ["--version", "-V", "--help", "-h"].includes(args[0]);
  if (
    !inspection && (process.env.COWBOY_EXECUTION_DESCRIPTOR ||
      await matrixConfiguration("claude"))
  ) {
    if (!isAbsolute(process.env.CLAUDE_CODE_EXECUTABLE ?? "")) {
      throw new Error("Missing exact Claude executable");
    }
    process.env[privateCli] = process.env.CLAUDE_CODE_EXECUTABLE;
    process.env.CLAUDE_CODE_EXECUTABLE = fileURLToPath(
      new URL("../bin/cowboy-configured-cli", import.meta.url),
    );
  }
  const upstream = fileURLToPath(
    new URL(
      "./node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js",
      import.meta.url,
    ),
  );
  process.argv = [process.execPath, upstream, ...args];
  await import(upstream);
}

if (
  process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main(process.argv.slice(2)).catch((error) => {
    process.stderr.write(
      (error.cowboyDiagnostic ??
        "Cowboy Claude execution binding failed; local fallback is disabled") +
        "\n",
    );
    process.exitCode = 1;
    process.stdin.destroy();
  });
}
