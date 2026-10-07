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
// Native's base hook input, as its classic events last reported it.
const hookBase = {};
// Native agent id -> its agent type, from SubagentStart.
const subagentTypes = new Map();

function recordEffort(_$, event, next) {
  if (event?.effort && typeof event.effort === "object") {
    hookBase.effort = event.effort;
  }
  return next(event);
}

export function recordHookBase(event) {
  for (const key of ["session_id", "transcript_path", "prompt_id"]) {
    if (typeof event?.[key] === "string") hookBase[key] = event[key];
  }
  if (typeof event?.permission_mode === "string") {
    hookBase.permission_mode = event.permission_mode;
  }
  if (event?.effort && typeof event.effort === "object") {
    hookBase.effort = event.effort;
  }
}

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

// Native background task id -> the target command it stands for.
const shellTasks = new Map();
// Target job id -> its native background task, and the calls in flight that
// start or stop one (they pass to native untouched).
const jobMirrors = new Map();
const mirrorCalls = new Set();

// A notification for a native task standing for a target command reads as
// one for that command: its id, tool use, output handle and command line.
export function targetShellNotification(notification, tasks) {
  const id = /<task-id>([^<]*)<\/task-id>/.exec(notification)?.[1];
  const task = tasks.get(id);
  if (!task) return notification;
  return notification
    .replace(`<task-id>${id}</task-id>`, `<task-id>${task.jobId}</task-id>`)
    .replace(
      /<tool-use-id>[^<]*<\/tool-use-id>/,
      `<tool-use-id>${task.toolUseId}</tool-use-id>`,
    )
    .replace(
      /<output-file>[^<]*<\/output-file>/,
      `<output-file>cowboy-task://${task.jobId}</output-file>`,
    )
    .replace(
      /<summary>Background command "[\s\S]*?" (completed|failed)/,
      (_match, outcome) =>
        `<summary>Background command "${task.command}" ${outcome}`,
    );
}

export function targetTaskNotificationText(text, outputs) {
  return text.replace(
    /<task-notification>[\s\S]*?<\/task-notification>/g,
    (found) => {
      const notification = targetShellNotification(found, shellTasks);
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

const PATH_KEYS = ["file_path", "notebook_path", "path"];
// Never a runtime working directory or a runtime-special location.
const OUTSIDE = "/.cowboy-target-outside";

// The target path a tool argument names: relative to the target workspace,
// with ".." and the target home resolved lexically (symlinks are not).
export function targetPath(value, { targetCwd, targetHome }) {
  let path = value;
  if (path === "~" || path.startsWith("~/")) {
    if (!targetHome) return null;
    path = targetHome + path.slice(1);
  } else if (!path.startsWith("/")) path = targetCwd + "/" + path;
  const parts = [];
  for (const part of path.split("/")) {
    if (part === "..") parts.pop();
    else if (part && part !== ".") parts.push(part);
  }
  return "/" + parts.join("/");
}

export function insideWorkspace(path, targetCwd) {
  return targetCwd === "/" || path === targetCwd ||
    path.startsWith(targetCwd + "/");
}

// Native permission evaluation resolves working directories on the runtime.
// Present target workspace paths at their runtime workspace equivalent, and
// every other target path under a root that native can never treat as its
// workspace, home or another special runtime location.
export function permissionInput(input, paths, outside = false) {
  const mapped = { ...input };
  for (const key of PATH_KEYS) {
    if (typeof mapped[key] !== "string") continue;
    const path = targetPath(mapped[key], paths);
    if (path === null) mapped[key] = OUTSIDE + "/~" + mapped[key].slice(1);
    else if (!outside && insideWorkspace(path, paths.targetCwd)) {
      const rest = paths.targetCwd === "/"
        ? path
        : path.slice(paths.targetCwd.length);
      mapped[key] = paths.runtimeCwd + rest;
    } else mapped[key] = OUTSIDE + path;
  }
  return mapped;
}

async function targetQuery($, path, value) {
  const response = await bridgePost($, path, value);
  return response.ok ? JSON.parse(response.text) : {};
}

// Native rules and mode decide first, through the engine's own check (no
// native body runs). Only an "ask" reaches the host's approval dialog. Native
// judges paths on its own filesystem; these follow the target's instead.
async function decide($, event, input) {
  const tool = event.tool;
  const check = await $.tool.check({
    tool,
    input: permissionInput(input, context),
  });
  // A rule naming a target path matches the arguments as written, as it
  // would locally. A deny under either reading refuses the call.
  const original = await $.tool.check({ tool, input });
  if (original?.decision === "deny") return original;
  if (check?.decision === "deny") return check;
  if (original?.rule) return original;
  const key = PATH_KEYS.find((name) => typeof input[name] === "string");
  const path = key && targetPath(input[key], context);
  // Native refuses, in every mode, to Write onto a symbolic link.
  if (tool === "Write" && typeof path === "string") {
    const link = await targetQuery($, "/link", { path });
    if (link.symlink !== false) {
      const real = link.symlink === true
        ? (await targetQuery($, "/resolve", { path })).path
        : null;
      return link.symlink === true && typeof real === "string"
        ? {
          decision: "deny",
          reason:
            `Refusing to write ${path}: it is a symbolic link. Write to the link's target path instead: ${real}.`,
        }
        : { decision: "ask", reason: `${path} could not be inspected.` };
    }
  }
  if (check?.decision !== "allow") return check;
  // Commands run unchanged. One naming the runtime workspace, which is not
  // the target's, was judged against the wrong project: ask only if native
  // would not also allow it with that path outside (e.g. outside bypass).
  if (
    tool === "Bash" && typeof input.command === "string" &&
    !insideWorkspace(context.runtimeCwd, context.targetCwd) &&
    input.command.includes(context.runtimeCwd)
  ) {
    const outside = await $.tool.check({
      tool,
      input: {
        ...input,
        command: input.command.split(context.runtimeCwd).join(
          OUTSIDE + context.runtimeCwd,
        ),
      },
    });
    if (outside?.decision === "allow") return check;
    return {
      decision: "ask",
      reason: "The command names a path outside the target workspace.",
    };
  }
  if (
    typeof path !== "string" || !insideWorkspace(path, context.targetCwd)
  ) return check;
  // Allowed only because it is inside the workspace? Natively the path is
  // resolved through symlinks first; an outside destination loses that.
  const outside = await $.tool.check({
    tool,
    input: permissionInput(input, context, true),
  });
  if (outside?.decision === "allow") return check;
  const real = (await targetQuery($, "/resolve", { path })).path;
  if (typeof real !== "string") {
    return { decision: "ask", reason: `${path} could not be resolved.` };
  }
  if (real === path) return check;
  const resolved = await $.tool.check({
    tool,
    input: permissionInput({ ...input, [key]: real }, context),
  });
  return resolved?.decision === "allow" ? check : {
    decision: resolved?.decision === "deny" ? "deny" : "ask",
    reason:
      `${path} resolves through a symlink to ${real}, which is outside the allowed working directories.`,
  };
}

// Native hook matchers: absent, empty or "*" match all; otherwise a
// case-sensitive regular expression matched against the whole tool name.
export function hookMatches(matcher, tool) {
  if (matcher === undefined || matcher === "" || matcher === "*") return true;
  try {
    return new RegExp("^(?:" + matcher + ")$").test(tool);
  } catch {
    return matcher === tool;
  }
}

// One tool hook's effect, as native reports it to the model. Non-zero exits
// other than 2, timeouts and unparsable output are non-blocking.
export function hookOutcome(event, tool, command, run) {
  if (!run || run.timedOut) return {};
  if (run.exitCode === 2) {
    // A PermissionRequest exit 2 decides nothing; the prompt still asks.
    if (event === "PermissionRequest") return {};
    return event === "PreToolUse"
      ? {
        deny:
          `PreToolUse:${tool} hook error: [${command}]: ${run.stderr.trim()}`,
      }
      : {
        context: [
          `${event}:${tool} hook blocking error from command: "${command}": [${command}]: ${run.stderr}`,
        ],
      };
  }
  if (run.exitCode !== 0) return {};
  let output;
  try {
    output = JSON.parse(run.stdout.trim());
  } catch {
    return {};
  }
  if (!output || typeof output !== "object" || Array.isArray(output)) {
    return {};
  }
  const specific = output.hookSpecificOutput ?? {};
  if (event === "PermissionRequest") {
    // Natively the first hook decision answers the pending prompt.
    const decision = specific.decision;
    if (decision?.behavior === "deny") {
      return {
        deny: typeof decision.message === "string"
          ? decision.message
          : "Permission denied by a project hook.",
        ...(decision.interrupt === true ? { stop: "" } : {}),
      };
    }
    if (decision?.behavior !== "allow") return {};
    return decision.updatedInput &&
        typeof decision.updatedInput === "object" &&
        !Array.isArray(decision.updatedInput)
      ? { allow: true, input: decision.updatedInput }
      : { allow: true };
  }
  const outcome = { context: [] };
  if (output.continue === false) outcome.stop = output.stopReason ?? "";
  if (typeof specific.additionalContext === "string") {
    outcome.context.push(
      `${event}:${tool} hook additional context: ${specific.additionalContext}`,
    );
  }
  if (event === "PostToolUse") {
    if (output.decision === "block") {
      outcome.context.unshift(
        `PostToolUse:${tool} hook blocking error from command: "${command}": ${
          output.reason ?? ""
        }`,
      );
    }
    return outcome;
  }
  if (event !== "PreToolUse") return outcome;
  const decision = specific.permissionDecision ??
    (output.decision === "block"
      ? "deny"
      : output.decision === "approve"
      ? "allow"
      : undefined);
  const reason = specific.permissionDecisionReason ?? output.reason ?? "";
  if (decision === "deny") {
    outcome.deny = `PreToolUse:${tool} hook error: ${reason}`;
  } else if (decision === "ask") outcome.ask = reason;
  else if (decision === "allow") outcome.allow = true;
  if (
    specific.updatedInput && typeof specific.updatedInput === "object" &&
    !Array.isArray(specific.updatedInput)
  ) outcome.input = specific.updatedInput;
  return outcome;
}

// `run` is the calling tool's lifecycle: its abandonment cancels this hook's
// target command too, and an abandoned call starts or awaits no hook.
async function runTargetHook($, entry, input, owner, run) {
  if (run.abandoned()) return undefined;
  const id = "hook-" + crypto.randomUUID();
  run.hooks.push(id);
  const { transcript_path: transcriptPath, ...rest } = input;
  let response = await bridgePost($, "/hook", {
    id,
    command: entry.command,
    ...(entry.argv ? { argv: entry.argv } : {}),
    input: rest,
    timeout: entry.timeout,
    ...(owner === undefined ? {} : { owner }),
    ...(typeof transcriptPath === "string" ? { transcriptPath } : {}),
  });
  while (response.ok && response.status === 202 && !run.abandoned()) {
    response = await bridgePost($, "/result", { id });
  }
  if (run.abandoned()) {
    run.abandon();
    return undefined;
  }
  const hook = response.ok ? JSON.parse(response.text).hook : undefined;
  // The bridge or target could not run it: not the hook's own outcome.
  return hook ?? { unavailable: true };
}

// Project PreToolUse/PostToolUse hooks for a facade tool, run on the target in
// parallel (identical commands once) and folded as native folds them.
async function toolHooks($, event, call, input, run, response, error) {
  const matching = (context.hooks.tool[event] ?? []).filter((group) =>
    hookMatches(group.matcher, call.tool)
  );
  const unsupported = matching.find((group) => group.unsupported);
  // A permission hook that cannot run decides nothing: the prompt still asks.
  if (unsupported && event !== "PermissionRequest") {
    return {
      context: [],
      deny:
        `The project's ${event} ${unsupported.unsupported} for ${call.tool} cannot run in this execution environment, so the call did not run.`,
    };
  }
  const entries = [
    ...new Map(
      matching.filter((group) => !group.unsupported).map((group) => {
        const entry = context.hooks.commands[group.index];
        return [entry.command, entry];
      }),
    ).values(),
  ];
  if (!entries.length) return { context: [] };
  let session;
  try {
    session = hookBase.session_id ?? await $.session.id();
  } catch {
    session = undefined;
  }
  const hookInput = {
    ...hookBase,
    ...(typeof session === "string" ? { session_id: session } : {}),
    cwd: context.targetCwd,
    ...(call.agentId === undefined ? {} : { agent_id: call.agentId }),
    ...(subagentTypes.has(call.agentId)
      ? { agent_type: subagentTypes.get(call.agentId) }
      : {}),
    hook_event_name: event,
    tool_name: call.tool,
    tool_input: input,
    // Native sends no suggestions for these calls (see permit) and no tool
    // use id to a PermissionRequest hook.
    ...(event === "PermissionRequest"
      ? { permission_suggestions: [] }
      : { tool_use_id: call.tool_use_id }),
    ...(response === undefined ? {} : { tool_response: response }),
    ...(error === undefined ? {} : { error }),
  };
  const runOne = (entry) =>
    runTargetHook($, entry, hookInput, call.agentId, run).catch(() => ({
      unavailable: true,
    }));
  // Async hooks run in the background, as natively: they cannot decide, and
  // their additionalContext/systemMessage reach the model on its next turn.
  for (const entry of entries.filter((entry) => entry.async)) {
    runOne(entry).then((result) => deliverAsync($, event, call, result))
      .catch(() => {});
  }
  const runs = await Promise.all(
    entries.filter((entry) => !entry.async).map(async (entry) => {
      const result = await runOne(entry);
      // A racing caller acts on each outcome as it arrives.
      run.each?.(hookOutcome(event, call.tool, entry.command, result));
      return { entry, result };
    }),
  );
  // A guard that could not run must not let the call proceed unchecked.
  const unavailable = runs.find(({ result }) => result?.unavailable);
  if (event === "PreToolUse" && unavailable) {
    return {
      context: [],
      deny:
        `PreToolUse:${call.tool} hook could not run on the target, so the call did not run.`,
    };
  }
  const outcomes = runs.map(({ entry, result }) =>
    hookOutcome(event, call.tool, entry.command, result)
  );
  const folded = { context: outcomes.flatMap((item) => item.context ?? []) };
  for (const key of ["deny", "ask", "allow", "input", "stop"]) {
    const found = outcomes.find((item) => item[key] !== undefined);
    if (found) folded[key] = found[key];
  }
  return folded;
}

export function asyncHookNotes(event, tool, run) {
  if (run?.exitCode !== 0) return [];
  let output;
  try {
    output = JSON.parse(run.stdout.trim());
  } catch {
    return [];
  }
  const notes = [];
  if (typeof output?.hookSpecificOutput?.additionalContext === "string") {
    notes.push(
      `${event}:${tool} async hook additional context: ${output.hookSpecificOutput.additionalContext}`,
    );
  }
  if (typeof output?.systemMessage === "string") {
    notes.push(`${event}:${tool} async hook: ${output.systemMessage}`);
  }
  return notes;
}

// A model-visible row for the next request; it starts no turn.
async function deliverAsync($, event, call, run) {
  const notes = asyncHookNotes(event, call.tool, run);
  if (!notes.length) return;
  await $.session.append({
    message: {
      type: "user",
      content: [{
        type: "text",
        text: `<system-reminder>\n${notes.join("\n")}\n</system-reminder>`,
      }],
    },
    ...(call.agentId === undefined ? {} : { agentId: call.agentId }),
  });
}

const NOTIFIED = "You will be notified when it completes. ";

// Natively a command left running notifies the model when it ends, into a
// running turn or as a turn of its own. A native background task running the
// runtime-local waiter stands for the target command, so native delivers that
// notification; without one, the result drops its promise.
async function notifyOnEnd($, event, task, result) {
  let nativeId;
  if (
    event.agentId === undefined && typeof context.taskWait === "string" &&
    /^[a-zA-Z0-9-]{1,128}$/.test(task.id) && typeof task.command === "string"
  ) {
    const command = `${context.taskWait} ${task.id}`;
    mirrorCalls.add(command);
    try {
      const started = await $.tool.call({
        tool: "Bash",
        command,
        run_in_background: true,
      });
      nativeId = started?.result?.backgroundTaskId;
    } catch {
      nativeId = undefined;
    } finally {
      mirrorCalls.delete(command);
    }
  }
  if (typeof nativeId !== "string" || shellTasks.size >= 4096) {
    return { ...result, stdout: result.stdout.replace(NOTIFIED, "") };
  }
  shellTasks.set(nativeId, {
    jobId: task.id,
    toolUseId: event.tool_use_id,
    command: task.command,
  });
  jobMirrors.set(task.id, nativeId);
  return result;
}

// Natively a stopped background command sends no notification: stop the
// native task standing for it too.
async function stopMirror($, jobId) {
  const nativeId = jobMirrors.get(jobId);
  if (nativeId === undefined) return;
  jobMirrors.delete(jobId);
  mirrorCalls.add(nativeId);
  try {
    await $.tool.call({ tool: "TaskStop", task_id: nativeId });
  } catch {
    // The task may already have ended; its notification then still reads as
    // the command's.
  } finally {
    mirrorCalls.delete(nativeId);
  }
}

// The session id and effort native puts in a Bash command's environment.
async function shellSession($) {
  let sessionId = hookBase.session_id;
  if (typeof sessionId !== "string") {
    try {
      sessionId = await $.session.id();
    } catch {
      sessionId = undefined;
    }
  }
  const effort = hookBase.effort?.level;
  return {
    ...(typeof sessionId === "string" && /^[a-zA-Z0-9-]{1,128}$/.test(sessionId)
      ? { sessionId }
      : {}),
    ...(typeof effort === "string" && /^[a-z]{1,16}$/.test(effort)
      ? { effort }
      : {}),
  };
}

// Project PermissionRequest hooks for a prompt the host has not answered.
// Natively they race the prompt and each other: the first decision withdraws
// it, and of decisions arriving together a deny wins (measured on 2.1.287).
// They have their own lifecycle: the host's answer cancels them without
// abandoning the call.
function permissionHooks($, event, input, run) {
  let done = false;
  const own = [];
  const decisions = [];
  const hookRun = {
    each: (outcome) => {
      if (outcome.deny !== undefined || outcome.allow) decisions.push(outcome);
    },
    hooks: { push: (id) => (own.push(id), run.hooks.push(id)) },
    abandoned: () => done || run.abandoned(),
    abandon: () => {
      if (run.abandoned()) return run.abandon();
      for (const id of own) bridgePost($, "/cancel", { id }).catch(() => {});
    },
  };
  toolHooks($, "PermissionRequest", event, input, hookRun).catch(() => {});
  return {
    decision: () =>
      decisions.find((item) => item.deny !== undefined) ?? decisions[0],
    settle: () => {
      if (done) return;
      done = true;
      hookRun.abandon();
    },
  };
}

async function permit($, event, input, run, hook = {}) {
  const { abandoned, abandon } = run;
  let check;
  try {
    check = await decide($, event, input);
  } catch {
    return { deny: "Permission check unavailable; the tool did not run." };
  }
  // A PreToolUse hook decides before the permission prompt, as natively:
  // "allow" skips it and "ask" requests it; a deny rule still refuses.
  if (check?.decision === "ask" && hook.allow) check = { decision: "allow" };
  if (check?.decision !== "deny" && hook.ask !== undefined) {
    check = { decision: "ask", reason: hook.ask };
  }
  if (check?.decision === "allow") return { input };
  if (check?.decision !== "ask") {
    return {
      deny: "Permission denied" + (check?.reason ? ": " + check.reason : "."),
    };
  }
  let hooks;
  try {
    for (;;) {
      const decided = hooks?.decision();
      if (decided) {
        await bridgePost($, "/withdraw", { id: event.tool_use_id });
        if (decided.stop !== undefined) $.turn.abort().catch(() => {});
        return decided.deny !== undefined
          ? { deny: decided.deny }
          : { input: decided.input ?? input };
      }
      const response = await bridgePost($, "/permission", {
        id: event.tool_use_id,
        tool: event.tool,
        input,
        reason: typeof check.reason === "string" ? check.reason : null,
        ...(event.agentId === undefined ? {} : { owner: event.agentId }),
      });
      if (abandoned()) {
        abandon();
        return { deny: "Tool call was cancelled" };
      }
      if (!response.ok) {
        return {
          deny: "Permission request unavailable; the tool did not run.",
        };
      }
      if (response.status === 202) {
        // The host is really prompting (dontAsk denies at once): start the
        // project's PermissionRequest hooks against it.
        hooks ??= permissionHooks($, event, input, run);
        continue;
      }
      return hostAnswer(JSON.parse(response.text), input);
    }
  } finally {
    hooks?.settle();
  }
}

function hostAnswer(result, input) {
  if (result.behavior !== "allow") {
    return {
      deny: typeof result.message === "string" && result.message
        ? result.message
        : "The user denied this tool use.",
    };
  }
  // The host may amend the call; native runs the updated input.
  const updated = result.updatedInput;
  return {
    input: updated && typeof updated === "object" && !Array.isArray(updated)
      ? updated
      : input,
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

export function unlabeledContext(text) {
  const label = "tool.call hook additional context: ";
  return typeof text === "string" && text.startsWith(label)
    ? text.slice(label.length)
    : text;
}

// Replaces native's `# gitStatus` section (the runtime's) with the target's,
// or drops it outside a repository; null when nothing else remains.
export function targetSessionContext(text, git) {
  const header = "# gitStatus\n";
  const trailer = "\n\nClaude Code attached this context automatically";
  const start = text.indexOf(header);
  if (start < 0) {
    // Native had no Git context (its runtime directory is no repository):
    // the target's still goes where native puts it.
    const at = text.indexOf(trailer);
    return git === null || at < 0
      ? text
      : `${text.slice(0, at)}\n${header}${git}${text.slice(at)}`;
  }
  let end = text.indexOf("\n# ", start + header.length);
  if (end < 0) end = text.indexOf(trailer, start);
  if (end < 0) end = text.length;
  const rest = text.slice(0, start) + (git === null ? "" : header + git) +
    text.slice(end);
  return /^# /m.test(rest) ? rest : null;
}

// Native renders the target's instruction files in its own framing; none of
// the runtime's are kept.
function instructions(_$, event) {
  if (!context) {
    return {
      blocks: [{ name: "claudeMd", text: unavailable }],
      instructionFiles: [],
    };
  }
  const claudeMd = event.blocks.find((block) => block.name === "claudeMd") ??
    { name: "claudeMd", text: "" };
  return {
    blocks: [
      claudeMd,
      ...event.blocks.filter((block) => block.name === "currentDate"),
    ],
    instructionFiles: context.instructionFiles,
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

  // The native task standing for a target command is part of a call the
  // session already decided; it is never put to the user again.
  on("tool.check", (_$, event, next) =>
    mirrorCalls.has(
        event.tool === "Bash"
          ? event.input?.command
          : event.tool === "TaskStop"
          ? event.input?.task_id
          : undefined,
      )
      ? { decision: "allow" }
      : next(event));
  on("tool.call", async ($, event, next) => {
    if (["TodoWrite", "AskUserQuestion"].includes(event.tool)) {
      return next(event);
    }
    // This module's own native background task for a target command, or its
    // TaskStop: native runs the runtime-local waiter itself.
    if (
      event.agentId === undefined &&
      mirrorCalls.has(event.tool === "Bash" ? event.command : event.task_id)
    ) return next(event);
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
    const requested = { ...event };
    delete requested.tool;
    delete requested.tool_use_id;
    delete requested.agentId;
    delete requested.consent;
    // An interrupted turn or stopped agent abandons this call. Cancel only
    // the target processes it started; the bridge also refuses late admission.
    let cancelled = false;
    const hooks = [];
    const abandon = () => {
      if (cancelled) return;
      cancelled = true;
      for (const id of [event.tool_use_id, ...hooks]) {
        bridgePost($, "/cancel", { id }).catch(() => {});
      }
    };
    const run = { abandoned, abandon, hooks };
    signal?.addEventListener("abort", abandon, { once: true });
    try {
      // A PostToolUse review that cannot run after the effect refuses first.
      const unsupported = ["PostToolUse", "PostToolUseFailure"].flatMap((
        name,
      ) =>
        (context.hooks.tool[name] ?? []).map((group) => ({ ...group, name }))
      ).find((group) =>
        group.unsupported && hookMatches(group.matcher, event.tool)
      );
      if (unsupported) {
        return {
          deny:
            `The project's ${unsupported.name} ${unsupported.unsupported} for ${event.tool} cannot run in this execution environment, so the call did not run.`,
        };
      }
      // Natively a PreToolUse continue:false still runs the tool, then ends
      // the turn (measured on 2.1.287); the stop is applied after it.
      const pre = await toolHooks($, "PreToolUse", event, requested, run);
      if (abandoned()) {
        abandon();
        return { deny: "Tool call was cancelled" };
      }
      if (pre.deny) return { deny: pre.deny };
      const permitted = await permit(
        $,
        event,
        pre.input ?? requested,
        run,
        pre,
      );
      if (permitted.deny) return permitted;
      const input = permitted.input;
      let path = "/tool";
      let body = {
        id: event.tool_use_id,
        tool: event.tool,
        input,
        ...(event.agentId === undefined ? {} : { owner: event.agentId }),
        ...(event.tool === "Bash" ? { shell: await shellSession($) } : {}),
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
        let result = JSON.parse(response.text);
        if (response.status !== 202) {
          if (result.result === undefined) {
            if (typeof result.deny !== "string") return result;
            // The target operation failed: the project's failure hooks run,
            // and their feedback follows the error the model reads.
            const failure = await toolHooks(
              $,
              "PostToolUseFailure",
              event,
              input,
              run,
              undefined,
              result.deny,
            );
            // A PreToolUse stop still ends the turn after a failed call;
            // natively a PostToolUseFailure continue:false does not.
            if (pre.stop !== undefined) $.turn.abort().catch(() => {});
            // PreToolUse context still reaches the model after a failure.
            const notes = [...pre.context, ...failure.context];
            return notes.length
              ? { deny: [result.deny, ...notes].join("\n\n") }
              : result;
          }
          const { task, instructions: nested, ...answered } = result;
          if (task) {
            answered.result = await notifyOnEnd(
              $,
              event,
              task,
              answered.result,
            );
          }
          if (event.tool === "TaskStop") {
            await stopMirror($, input.task_id ?? input.shell_id);
          }
          result = answered;
          const post = await toolHooks(
            $,
            "PostToolUse",
            event,
            input,
            run,
            result.result,
          );
          // Nested instruction files follow the result, as natively.
          const reminders = [
            ...pre.context,
            ...(Array.isArray(nested)
              ? nested.map((file) =>
                `Contents of ${file.path}:\n\n${file.content}`
              )
              : nested?.unavailable
              ? [
                "The project's instruction files (CLAUDE.md and rules) for this file's directories could not be loaded from the target. Read them before relying on this file.",
              ]
              : []),
            ...post.context,
          ];
          // Natively the tool still ran; continue:false then ends the turn.
          const stop = pre.stop ?? post.stop;
          if (stop !== undefined) $.turn.abort().catch(() => {});
          return reminders.length ? { ...result, context: reminders } : result;
        }
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
  // This module's tool.call context stands for native's own reminders (hook
  // output, nested instruction files): it reads without the chain's label.
  on(
    "prompt.attachment",
    { origin: { kind: "plugin" } },
    (_$, event, next) => next({ ...event, text: unlabeledContext(event.text) }),
  );
  // Native's session context describes the runtime checkout; its Git status
  // becomes the target's, in native's own framing.
  on("prompt.attachment", { type: "session_context" }, (_$, event) => ({
    text: targetSessionContext(event.text, context ? context.git : null),
  })).catch(() => ({ text: null }));
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
  // Every classic event carries native's base hook input, hooks or not.
  on("classic.SessionStart", (_$, event, next) => {
    recordHookBase(event);
    return next(event);
  });
  on("classic.UserPromptSubmit", (_$, event, next) => {
    recordHookBase(event);
    return next(event);
  });
  // Native reports the turn's effort only in tool-context hook input, first
  // after a tool batch; Bash and tool hooks carry it from then on.
  on("classic.PostToolBatch", recordEffort);
  on("classic.Stop", recordEffort);
  // A subagent's tool hooks name its type, as natively.
  on("classic.SubagentStart", (_$, event, next) => {
    if (
      typeof event?.agent_id === "string" &&
      typeof event.agent_type === "string" && subagentTypes.size < 4096
    ) subagentTypes.set(event.agent_id, event.agent_type);
    return next(event);
  });
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
      ![loaded.targetCwd, loaded.runtimeCwd].every((value) =>
        typeof value === "string" && value.startsWith("/") &&
        (value === "/" || !value.endsWith("/"))
      ) ||
      !loaded.hooks || !Array.isArray(loaded.hooks.commands) ||
      !loaded.hooks.tool || typeof loaded.hooks.tool !== "object" ||
      !(loaded.targetHome === null ||
        (typeof loaded.targetHome === "string" &&
          loaded.targetHome.startsWith("/"))) ||
      typeof loaded.environment !== "string" ||
      loaded.environment.length > 262144 ||
      !(loaded.git === null ||
        (typeof loaded.git === "string" && loaded.git.length <= 262144)) ||
      !Array.isArray(loaded.instructionFiles) ||
      !loaded.instructionFiles.every((file) =>
        typeof file?.path === "string" && file.path.startsWith("/") &&
        ["user", "project", "local"].includes(file.kind) &&
        typeof file.content === "string" &&
        (file.parent === undefined || typeof file.parent === "string")
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
