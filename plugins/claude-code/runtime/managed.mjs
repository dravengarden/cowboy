// Cowboy managed read-only calls. The signed managed profile passes
// MANAGED_PROFILE_FLAG; the constraint itself comes from the bound execution
// target, whose keeper enforces it on every command. This side keeps the
// native session within that profile and applies the round's output schema
// as native structured output.
import { randomUUID } from "node:crypto";
import { isAbsolute } from "node:path";

export const MANAGED_PROFILE_FLAG = "--cowboy-managed-profile=read-only-v1";
export const MANAGED_PROFILE_ENV = "COWBOY_MANAGED_PROFILE";
const PROFILE = "read_only_v1";
const MAX_ROUND_BYTES = 256 * 1024;

// Native tools a read-only reviewer keeps. Writes, nested agents, questions
// for an absent user, web access from the runtime and skills are removed;
// the keeper refuses target writes even if one were reached.
export const MANAGED_TOOLS = ["Bash", "Read", "Glob", "Grep"];
export const MANAGED_PASSTHROUGH = [
  "TaskCreate",
  "TaskGet",
  "TaskList",
  "TaskUpdate",
];
export const MANAGED_DISALLOWED = [
  "Write",
  "Edit",
  "NotebookEdit",
  "TaskStop",
  "Agent",
  "SendMessage",
  "AskUserQuestion",
  "WebFetch",
  "WebSearch",
  "Skill",
  "ReportFindings",
];

// The adapter's own arguments without the profile flag, and whether it was
// present. Any other spelling of a managed profile is refused.
export function managedArguments(args) {
  const rest = args.filter((argument) => argument !== MANAGED_PROFILE_FLAG);
  if (
    rest.some((argument) => argument.startsWith("--cowboy-managed-profile"))
  ) {
    throw new Error("Unsupported Cowboy managed profile");
  }
  return { managed: rest.length !== args.length, args: rest };
}

// The target's announcement must name exactly the profile this generation
// launched with; a missing one means nothing enforces it there.
export function managedConstraint(binding, announced) {
  if (
    binding?.managed?.profile !== PROFILE ||
    typeof binding.managed.parent_session_id !== "string" ||
    announced?.schema !== 1 || announced.profile !== PROFILE ||
    typeof announced.roundPath !== "string" ||
    !isAbsolute(announced.roundPath)
  ) {
    throw new Error(
      "A managed Claude call requires its read-only execution environment",
    );
  }
  return { roundPath: announced.roundPath };
}

// The Machine-written round. Only its output schema shapes the native turn.
export function parseRound(bytes) {
  if (!bytes || bytes.length > MAX_ROUND_BYTES) {
    throw new Error("Managed call round is unavailable");
  }
  const round = JSON.parse(bytes.toString("utf8"));
  if (
    round?.schema !== 1 || typeof round.call_id !== "string" ||
    (round.output_schema !== undefined &&
      (round.output_schema === null ||
        typeof round.output_schema !== "object" ||
        Array.isArray(round.output_schema)))
  ) {
    throw new Error("Managed call round is invalid");
  }
  return round;
}

// Native argv for a managed turn. Approval prompts would wait for nobody:
// the listed read tools are allowed and everything else is denied.
export function managedNativeArguments(argv, schema) {
  const replaced = new Set([
    "--permission-mode",
    "--json-schema",
    "--allowedTools",
  ]);
  const flags = new Set([
    "--allow-dangerously-skip-permissions",
    "--dangerously-skip-permissions",
  ]);
  const kept = [];
  for (let index = 0; index < argv.length; index++) {
    const argument = argv[index];
    if (replaced.has(argument)) {
      index++;
    } else if (argument === "--tools") {
      kept.push(argument, [...MANAGED_TOOLS, ...MANAGED_PASSTHROUGH].join(","));
      index++;
    } else if (argument === "--disallowedTools") {
      const disallowed = new Set([
        ...String(argv[++index] ?? "").split(",").filter(Boolean),
        ...MANAGED_DISALLOWED,
      ]);
      kept.push(argument, [...disallowed].join(","));
    } else if (!flags.has(argument)) kept.push(argument);
  }
  return [
    ...kept,
    "--permission-mode",
    "dontAsk",
    "--allowedTools",
    [...MANAGED_TOOLS, ...MANAGED_PASSTHROUGH].join(","),
    ...(schema ? ["--json-schema", JSON.stringify(schema)] : []),
  ];
}

// Native reports a validated structured result on its result frame. The
// pinned ACP adapter forwards only message text, so the result reaches
// Cowboy as one final top-level message carrying exactly that JSON.
export function structuredMessage(result, lastAssistant) {
  if (
    result?.type !== "result" || result.subtype !== "success" ||
    result.structured_output === undefined
  ) {
    return undefined;
  }
  const message = lastAssistant?.message ?? {};
  return {
    type: "assistant",
    uuid: randomUUID(),
    session_id: result.session_id ?? lastAssistant?.session_id ?? "",
    parent_tool_use_id: null,
    message: {
      id: "msg_cowboy_structured_" + randomUUID().replaceAll("-", ""),
      type: "message",
      role: "assistant",
      model:
        typeof message.model === "string" && message.model !== "<synthetic>"
          ? message.model
          : "claude",
      content: [{
        type: "text",
        text: JSON.stringify(result.structured_output),
      }],
      stop_reason: "end_turn",
      stop_sequence: null,
      ...(message.usage ? { usage: message.usage } : {}),
    },
  };
}

// Control a managed child's configuration may not change.
export function managedControlRefused(request) {
  return request?.subtype === "set_permission_mode";
}

// A managed turn's instruction is ordinary text, never a native command.
export function managedCommandRefused(prompt) {
  return /^\s*\//.test(prompt);
}
