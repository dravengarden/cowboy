// Cowboy managed read-only calls for Antigravity (context asks only: the
// Controller admits Antigravity as a target of `analysis` and `sessions`
// asks, with text output). The constraint comes from the execution target:
// the binding names the managed profile and the target's keeper announces
// the one it enforces on every native request (read-only, network-free
// process starts; no file writes or HTTP). This side keeps `agy` within
// that profile: only the facade's read tools and RunCommand exist, no
// Cowboy MCP, folder MCP or memory is attached, `agy`'s own web, question,
// subagent and schedule tools are denied, and every turn must carry the
// Machine-written round's prompt metadata.
//
// An account-sandbox runtime unit starts `bin/agy-proxy` with the command
// line the daemon fixes, so the signed managed profile's arguments are a
// declaration only there; the proxy learns managed mode from its execution
// descriptor and target, and tells the `PreToolUse` hook through
// HOOK_PROFILE_ENV in `agy`'s closed environment: `ordinary` for an ordinary
// session, `read-only-v1` for a managed child. The hook applies the ordinary
// policy only for the exact `ordinary` marker; a missing or unknown marker
// (`agy` not passing its environment on, say) gets the managed allow-list.
import { isAbsolute } from "node:path";

export const MANAGED_PROFILE_FLAG = "--cowboy-managed-profile=read-only-v1";
/** Kept in step with the Controller's `OUTPUT_CONTRACT_FLAG`. */
export const OUTPUT_CONTRACT_FLAG = "--cowboy-output-contract=v1";
/** The hook policy marker the proxy sets in `agy`'s environment. */
export const HOOK_PROFILE_ENV = "COWBOY_HOOK_PROFILE";
export const HOOK_PROFILE_ORDINARY = "ordinary";
export const HOOK_PROFILE_MANAGED = "read-only-v1";
const PROFILE = "read_only_v1";
/** The prompt `_meta` key of a managed round (src/managed_calls/round.rs). */
export const MANAGED_META_KEY = "cowboy.dev/managedCall";

/** Facade tools a managed child keeps; writes and StopCommand are not offered. */
export const MANAGED_FACADE_TOOLS = Object.freeze([
  "ReadFile",
  "ListDirectory",
  "SearchFiles",
  "RunCommand",
  "UpdatePlan",
]);
/** The same tools as `agy` names them; nothing else may run. */
export const MANAGED_ALLOWED_TOOLS = Object.freeze(
  MANAGED_FACADE_TOOLS.map((name) => `mcp_cowboy_${name}`),
);
const MANAGED_ALLOWED = new Set(MANAGED_ALLOWED_TOOLS);
/** Configuration a managed child may change: the signed presets' values. */
export const MANAGED_CONFIG_OPTIONS = Object.freeze(["model", "thought_level"]);

export const MANAGED_DENIAL_REASON =
  "This is a read-only Cowboy managed call. Only mcp_cowboy_ReadFile, mcp_cowboy_ListDirectory, " +
  "mcp_cowboy_SearchFiles, mcp_cowboy_RunCommand (read-only, without network) and " +
  "mcp_cowboy_UpdatePlan are available; answer from what they show.";

export const MANAGED_ROUTING = [
  "This is a read-only Cowboy managed call: answer the request below from the workspace and",
  "reply with text. The workspace is on another machine; use only the tools of the `cowboy`",
  "MCP server: mcp_cowboy_ReadFile, mcp_cowboy_ListDirectory, mcp_cowboy_SearchFiles and",
  "mcp_cowboy_RunCommand (read-only and without network: commands cannot change files). Every",
  "other tool, including web search, URL reading, questions, subagents and Cowboy calls, is",
  "denied; do not try them. Nobody can answer a question during this call. Paths are relative",
  "to the working directory below or absolute anywhere the user can read. The local directory",
  "of this process is not the workspace.",
].join("\n");

/**
 * The adapter's arguments without the profile flag, and whether it was
 * present. Any other spelling of a managed profile is refused.
 */
export function managedArguments(args) {
  const profile = args.filter((argument) => argument !== MANAGED_PROFILE_FLAG);
  const managed = profile.length !== args.length;
  if (profile.some((argument) => argument.startsWith("--cowboy-managed-profile"))) {
    throw new Error("Unsupported Cowboy managed profile");
  }
  // The signed declaration that this adapter enforces output schemas; the
  // Controller reads it, `agy` never sees it.
  const rest = profile.filter((argument) => argument !== OUTPUT_CONTRACT_FLAG);
  if (rest.some((argument) => argument.startsWith("--cowboy-output-contract"))) {
    throw new Error("Unsupported Cowboy output contract");
  }
  if (rest.length !== profile.length && !managed) {
    throw new Error("The Cowboy output contract needs the managed profile");
  }
  return { managed, args: rest };
}

/**
 * Whether the hook running in `environment` applies the managed allow-list:
 * always, unless the proxy marked an ordinary session exactly. A missing,
 * empty or unknown marker never widens the policy.
 */
export function hookManaged(environment) {
  return environment?.[HOOK_PROFILE_ENV] !== HOOK_PROFILE_ORDINARY;
}

/** The hook marker of `agy`'s environment for a session of this kind. */
export function hookEnvironment(managed) {
  return { [HOOK_PROFILE_ENV]: managed ? HOOK_PROFILE_MANAGED : HOOK_PROFILE_ORDINARY };
}

/**
 * Whether this unit serves a managed child. The binding and the target's
 * announcement must agree on the exact profile; one without the other
 * means nothing enforces it, which is refused.
 */
export function managedMode(binding, announced) {
  const bound = binding?.managed !== undefined && binding?.managed !== null;
  const enforced = announced !== undefined && announced !== null;
  if (!bound && !enforced) return false;
  if (
    !validManagedBinding(binding) ||
    announced?.schema !== 1 ||
    announced.profile !== PROFILE ||
    typeof announced.roundPath !== "string" ||
    !isAbsolute(announced.roundPath)
  ) {
    throw new Error("A managed Antigravity call requires its read-only execution environment");
  }
  return true;
}

function validManagedBinding(binding) {
  const managed = binding?.managed;
  return (
    managed?.profile === PROFILE &&
    typeof managed.parent_session_id === "string" &&
    managed.parent_session_id.length > 0
  );
}

/**
 * Whether this unit's binding names a managed child, checked from what is
 * available before anything starts: the binding's own managed record must
 * name the exact profile and its parent, and a launch with the managed
 * profile flag needs a managed binding. Throws on any disagreement; the
 * target's announcement is checked once connected (`managedMode`).
 */
export function launchManagedMode(binding, launchManaged) {
  const bound = binding?.managed !== undefined && binding?.managed !== null;
  if (bound && !validManagedBinding(binding)) {
    throw new Error("A managed Antigravity call requires its read-only execution environment");
  }
  if (launchManaged && !bound) {
    throw new Error("A managed Antigravity profile requires a managed execution binding");
  }
  return bound;
}

/** Whether a managed child's `agy` may run the tool named `name`. */
export function managedAllowedTool(name) {
  return typeof name === "string" && MANAGED_ALLOWED.has(name);
}

const isObject = (value) => value !== null && typeof value === "object" && !Array.isArray(value);

/** The text blocks of an ACP prompt, joined. */
function promptWords(prompt) {
  if (!Array.isArray(prompt)) return "";
  return prompt
    .filter((block) => block?.type === "text" && typeof block.text === "string")
    .map((block) => block.text)
    .join("\n");
}

/**
 * Admit one `session/prompt` of a managed child. Returns `{ error }` for a
 * refused turn or `{ params }`: the request without Cowboy's round
 * metadata, which `agy` never sees, and the round's output schema, if any.
 * A round must be present, and a turn's instruction is never a native
 * slash command.
 */
export function managedPrompt(params) {
  const round = params?._meta?.[MANAGED_META_KEY];
  if (!isObject(round) || typeof round.callId !== "string" || !round.callId) {
    return { error: "A managed Antigravity turn requires its Cowboy managed-call round" };
  }
  const prompt = params.prompt;
  if (
    !Array.isArray(prompt) ||
    prompt.length === 0 ||
    !prompt.every(
      (block) =>
        isObject(block) &&
        block.type === "text" &&
        typeof block.text === "string" &&
        Object.keys(block).every((key) => key === "type" || key === "text" || key === "_meta"),
    )
  ) {
    return { error: "A managed Antigravity turn takes text blocks only" };
  }
  const schema = round.outputSchema ?? undefined;
  if (schema !== undefined && !isObject(schema)) {
    return { error: "A managed Antigravity round's outputSchema must be a JSON Schema object" };
  }
  if (/^\s*\//.test(promptWords(prompt))) {
    return { error: "A managed Antigravity turn cannot run a slash command" };
  }
  const { [MANAGED_META_KEY]: _round, ...meta } = params._meta;
  const next = { ...params };
  if (Object.keys(meta).length) next._meta = meta;
  else delete next._meta;
  // A structured round is answered through the proxy's output contract
  // (`managed-output.mjs`).
  return schema === undefined ? { params: next } : { params: next, schema };
}

/** Whether a prompt carries a managed round (refused outside managed mode). */
export function hasManagedRound(params) {
  return isObject(params?._meta) && Object.hasOwn(params._meta, MANAGED_META_KEY);
}

/**
 * Whether a managed child may send this configuration request: a model or
 * a preset option, never a mode (a mode could stop or widen tool use).
 */
export function managedConfigAllowed(method, params) {
  if (method === "session/set_model") return true;
  if (method === "session/set_config_option") {
    return MANAGED_CONFIG_OPTIONS.includes(params?.configId);
  }
  return method !== "session/set_mode";
}
