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
import { dirname, isAbsolute, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  bindingKey,
  DESCRIPTIONS,
  NATIVE_TOOLS,
  WorkspaceTools,
} from "./tools.mjs";
import { startModBridge } from "./mod-bridge.mjs";

const privateCli = "COWBOY_PRIVATE_CLAUDE_EXECUTABLE";
const forbiddenTools = [
  "Agent",
  "Task",
  "Skill",
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

export function nativeArguments(args, plugin) {
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
    "--permission-mode",
    "bypassPermissions",
    "--tools",
    [...NATIVE_TOOLS, "TodoWrite", "AskUserQuestion"].join(","),
    "--disallowedTools",
    [...disallowed].join(","),
    "--setting-sources",
    "",
    "--strict-mcp-config",
    "--plugin-dir",
    plugin,
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

export function initializeRequest(frame) {
  return {
    ...frame,
    request: {
      subtype: "initialize",
      sdkMcpServers: [],
      toolAliases: {},
      excludeDynamicSections: true,
      skills: [],
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

async function bridge(child, tools, context) {
  let resolveReady, rejectReady;
  const ready = new Promise((resolve, reject) => {
    resolveReady = resolve;
    rejectReady = reject;
  });
  ready.catch(() => {});
  let initial;
  let initialReply;
  let stage = "initialize";
  const checkId = randomUUID();
  const privateCommand = "cowboy-execution-ready-" + context.nonce;
  const timeout = setTimeout(
    () => rejectReady(new Error("Claude execution module did not initialize")),
    30000,
  );
  const cleanCommands = (frame) => {
    const result = structuredClone(frame);
    for (const value of [result, result.response?.response]) {
      if (Array.isArray(value?.commands)) {
        value.commands = value.commands.filter((command) =>
          !command.name.startsWith("cowboy-execution-ready-")
        );
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
        initial = initializeRequest(frame);
        await send(child.stdin, initial);
        continue;
      }
      await ready;
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
        if (subtype === "interrupt") await tools.cancelForeground();
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
        const command = /^\/([a-z][a-z0-9-]*)(?:\s|$)/i.exec(prompt.trim());
        if (
          command &&
          !["compact", "cost", "context", "status", "help"].includes(command[1])
        ) {
          throw new Error(
            "This command is unavailable in an execution session",
          );
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
      if (stage === "ready") await send(process.stdout, cleanCommands(frame));
      else if (frame.type === "control_request") {
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
    throw new Error("Bound Claude execution descriptor missing");
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
  let child, connection, modBridge;
  try {
    connection = await Connection.open(descriptor);
    const tools = new WorkspaceTools(
      connection,
      descriptor.binding,
      join(root, "state.json"),
    );
    await tools.load();
    const context = await tools.context();
    modBridge = await startModBridge(tools);
    context.socketPath = modBridge.socketPath;
    context.bridgeToken = modBridge.token;
    context.descriptions = DESCRIPTIONS;
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
    const contextPath = join(stage, "context.json");
    await writeFile(contextPath, JSON.stringify(context), {
      mode: 0o600,
      flag: "wx",
    });
    const environment = {
      ...process.env,
      COWBOY_CLAUDE_CONTEXT: contextPath,
      CLAUDE_CODE_DISABLE_AUTO_MEMORY: "1",
      // Native tool bodies never run for project operations. Keep implicit
      // attachments and local checkpoints disabled; Mods projects target context.
      CLAUDE_CODE_DISABLE_ATTACHMENTS: "1",
      CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS: "1",
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
    child = spawn(executable, nativeArguments(args, plugin), {
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
    bridge(child, tools, context).catch((error) => {
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
  if (process.env.COWBOY_EXECUTION_DESCRIPTOR) {
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
