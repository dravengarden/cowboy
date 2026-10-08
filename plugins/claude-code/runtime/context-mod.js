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
      /<summary>Background command "[\s\S]*?" (completed|failed|was stopped)/,
      (_match, outcome) =>
        `<summary>Background command "${task.command}" ${outcome}`,
    );
}

// Native task ids whose notification says native stopped them at their
// background deadline; the target commands they stand for stop with them.
export function deadlineTasks(text) {
  return [
    ...String(text).matchAll(
      /<task-notification>[\s\S]*?<\/task-notification>/g,
    ),
  ].filter(([found]) =>
    /<status>killed<\/status>/.test(found) &&
    /was stopped after reaching its background time limit/.test(found)
  ).map(([found]) => /<task-id>([^<]*)<\/task-id>/.exec(found)?.[1]).filter(
    (id) => shellTasks.has(id),
  );
}

// A target command native stopped at its deadline ends before the model
// reads that it was stopped. Returns the jobs whose stop the target did not
// confirm.
async function stopAtDeadline($, event) {
  if (event.origin?.kind !== "task-notification") return [];
  const content = event.message?.content;
  const text = typeof content === "string"
    ? content
    : Array.isArray(content)
    ? content.map((block) => block.text ?? "").join("\n")
    : "";
  const unconfirmed = [];
  for (const nativeId of deadlineTasks(text)) {
    const jobId = shellTasks.get(nativeId).jobId;
    jobMirrors.delete(jobId);
    const response = await bridgePost($, "/task-deadline", { id: jobId })
      .catch(() => undefined);
    let stopped = false;
    try {
      stopped = response?.ok && JSON.parse(response.text).stopped === true;
    } catch {
      stopped = false;
    }
    if (!stopped) unconfirmed.push(jobId);
  }
  return unconfirmed;
}

// A deadline notification whose target stop is unconfirmed says so.
export function unconfirmedStop(event, jobs) {
  if (!jobs.length) return event;
  const note = (text) =>
    text.replace(
      /(<task-id>([^<]*)<\/task-id>[\s\S]*?)(<\/task-notification>)/g,
      (whole, body, id, end) =>
        jobs.includes(id)
          ? `${body}<note>The target did not confirm that this command stopped; read cowboy-task://${id} before relying on it.</note>\n${end}`
          : whole,
    );
  const content = event.message.content;
  return {
    ...event,
    message: {
      ...event.message,
      content: typeof content === "string"
        ? note(content)
        : content.map((block) =>
          block.type === "text" ? { ...block, text: note(block.text) } : block
        ),
    },
  };
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
const DEADLINE =
  "If it is still running after 30m in the background, it will be stopped and you will be notified. ";

// Natively a command left running notifies the model when it ends, into a
// running turn or as a turn of its own. A native background task running the
// runtime-local waiter stands for the target command, so native delivers that
// notification; without one, the result drops its promise.
async function notifyOnEnd($, event, task, result, input = event, next) {
  let nativeId;
  if (
    typeof context.taskWait === "string" &&
    /^[a-zA-Z0-9-]{1,128}$/.test(task.id) && typeof task.command === "string"
  ) {
    const command = `${context.taskWait} ${task.id}`;
    // Native gives the task standing for it the command's own background
    // deadline: the requested timeout (as hooks and approvals amended it),
    // else 30 minutes.
    const deadline = input.run_in_background === true &&
        Number.isSafeInteger(input.timeout) && input.timeout > 0
      ? { timeout: input.timeout }
      : {};
    mirrorCalls.add(command);
    try {
      let started;
      if (event.agentId === undefined) {
        started = await $.tool.call({
          tool: "Bash",
          command,
          run_in_background: true,
          ...deadline,
        });
      } else {
        // A plugin's own call belongs to the main session (measured). The
        // agent's call itself, run natively as the task, makes it the
        // agent's: native notifies the agent, resuming it if it ended.
        const { timeout: _timeout, description: _description, ...rest } = event;
        started = await next({
          ...rest,
          command,
          run_in_background: true,
          ...deadline,
        });
      }
      nativeId = started?.result?.backgroundTaskId;
    } catch {
      nativeId = undefined;
    } finally {
      mirrorCalls.delete(command);
    }
  }
  if (typeof nativeId !== "string" || shellTasks.size >= 4096) {
    return {
      ...result,
      stdout: result.stdout.replace(NOTIFIED, "").replace(DEADLINE, ""),
    };
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

// Native's own Read of the private local copy of a target file (an image):
// its result, or its refusal naming the target file instead of the copy.
export async function nativeLocalRead($, event, next, answer) {
  const { localRead, localTarget, ...rest } = answer;
  let local;
  try {
    local = await next({ ...event, file_path: localRead });
  } catch (error) {
    local = { deny: String(error?.message ?? error).replace(/^Error: /, "") };
  } finally {
    try {
      await bridgePost($, "/local-release", { path: localRead });
    } catch {
      // The launcher removes leftover copies when it next starts.
    }
  }
  const named = JSON.parse(
    // A callback: the target path's own `$` sequences stay literal.
    JSON.stringify(local ?? {}).replaceAll(
      JSON.stringify(localRead).slice(1, -1),
      () => JSON.stringify(localTarget).slice(1, -1),
    ),
  );
  // A native failure comes back as its message, in the error's form.
  if (named.isError === true || typeof named.deny === "string") {
    const message = typeof named.deny === "string"
      ? named.deny
      : typeof named.result === "string"
      ? named.result
      : "Target image could not be read";
    return { deny: message.replace(/^Error: /, "") };
  }
  if (named.result === undefined) {
    return { deny: "Target image could not be read" };
  }
  // Whatever else native returns with it (its size note) stays.
  return { ...rest, ...named };
}

// Attachments native builds from this machine's files, editors and memory:
// they would describe the runtime, not the target. Target instruction files
// come from this module; the rest are absent.
export const RUNTIME_ATTACHMENTS = new Set([
  "file",
  "directory",
  "pdf_reference",
  "already_read_file",
  "compact_file_reference",
  "at_mention_reference",
  "edited_text_file",
  "edited_image_file",
  "nested_memory",
  "dynamic_skill",
  "diagnostics",
  "lsp_diagnostics",
  "selected_lines_in_ide",
  "selected_lines_in_diff",
  "opened_file_in_ide",
  "plan_file_reference",
  "relevant_memories",
  "account_memory_recall",
  "memory_update",
  "memory_saved",
]);

// Text about the target's skills under their native names and directories
// (launch.mjs loads them from a private plugin; see skills.mjs).
export function targetSkillText(text, skills) {
  if (typeof text !== "string" || !skills?.entries.length) return text;
  // One pass, longest first: a skill named like the start of another's name
  // never claims its directory, and a mapped path is not mapped again.
  const directories = new Map(
    skills.entries.filter((entry) => entry.mirrorDirectory).map((entry) => [
      entry.mirrorDirectory,
      entry.targetDirectory,
    ]),
  );
  const result = directories.size
    ? text.replace(
      new RegExp(
        [...directories.keys()].sort((left, right) =>
          right.length - left.length
        ).map((path) => path.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join(
          "|",
        ),
        "g",
      ),
      (path) => directories.get(path),
    )
    : text;
  return result.replaceAll(skills.prefix, "");
}

// Native's skill listing as a local session shows it: the target's user
// skills, project skills, then commands, by name; a bundled skill a target
// skill shadows is not listed. Native renders the stored listing again for
// later requests, so an already projected listing is left as it is.
export function targetSkillListing(text, skills) {
  if (typeof text !== "string" || !skills?.entries.length) return text;
  const [head, ...items] = text.split("\n- ");
  // Over native's budget, an entry may be its name alone.
  const named = (item) =>
    item.includes(": ") ? item.slice(0, item.indexOf(": ")) : item.trimEnd();
  const own = new Map(skills.entries.map((entry) => [entry.name, entry]));
  const rank = (entry) =>
    entry.kind === "command" ? 2 : entry.scope === "user" ? 0 : 1;
  const target = items.filter((item) => named(item).startsWith(skills.prefix))
    .map((item) => ({
      item: targetSkillText(item, skills),
      entry: own.get(named(item).slice(skills.prefix.length)),
    })).filter(({ entry }) => entry).sort((left, right) =>
      rank(left.entry) - rank(right.entry) ||
      (left.entry.name < right.entry.name
        ? -1
        : left.entry.name > right.entry.name
        ? 1
        : 0)
    );
  const shadowed = new Set(target.map(({ entry }) => entry.name));
  const rest = items.filter((item) =>
    !named(item).startsWith(skills.prefix) && !shadowed.has(named(item))
  );
  return [head, ...target.map(({ item }) => item), ...rest].join("\n- ");
}

// Whether a skill's allowed-tools grant a single plain command. A compound
// command or one with expansions is left to the session's own rules.
export function skillAllows(command, rules) {
  if (/[;&|<>`$(){}\n\\]/.test(command)) return false;
  return rules.some((rule) => {
    const match = /^Bash(?:\((.*)\))?$/.exec(rule.trim());
    if (!match) return false;
    const pattern = match[1];
    if (pattern === undefined || pattern === "" || pattern === "*") return true;
    if (pattern.endsWith(":*")) {
      const prefix = pattern.slice(0, -2);
      return command === prefix || command.startsWith(prefix + " ");
    }
    if (pattern.endsWith("*")) return command.startsWith(pattern.slice(0, -1));
    return command === pattern;
  });
}

// Failures of a skill's shell commands, for the Skill call that loaded it.
// One Skill call per skill at a time: native's expansion event names only
// the skill, and this ties it to its call (agent and cancellation).
const skillCalls = new Map();
const skillTurns = new Map();
let skillShellCount = 0;

// One `!` command of a target skill, run by the target's Bash as native runs
// it locally: after the permission check, without tool hooks. It belongs to
// the Skill call that loaded the skill (its agent, its cancellation). Returns
// the text that replaces it, or the skill's failure.
async function skillShell($, command, raw, entry, call = {}) {
  const decision = await decide($, { tool: "Bash" }, { command });
  if (
    decision?.decision !== "allow" &&
    !(decision?.decision === "ask" && skillAllows(command, entry.allowedTools))
  ) {
    return {
      failure: `Shell command permission check failed for pattern "${raw}": ${
        decision?.reason || "Permission denied"
      }`,
    };
  }
  const id = `${context.nonce}-skill-${++skillShellCount}`;
  const abandoned = () =>
    call.signal?.aborted ||
    (call.owner !== undefined && stoppedAgents.has(call.owner));
  const abandon = () => bridgePost($, "/cancel", { id }).catch(() => {});
  if (abandoned()) return { failure: "Tool call was cancelled" };
  call.signal?.addEventListener("abort", abandon, { once: true });
  let path = "/tool";
  let body = {
    id,
    tool: "Bash",
    input: { command },
    ...(call.owner === undefined ? {} : { owner: call.owner }),
    shell: await shellSession($),
  };
  try {
    for (;;) {
      const response = await bridgePost($, path, body);
      if (abandoned()) {
        abandon();
        return { failure: "Tool call was cancelled" };
      }
      if (!response.ok) {
        return { failure: `Shell command failed for pattern "${raw}":` };
      }
      const result = JSON.parse(response.text);
      if (response.status === 202) {
        path = "/result";
        body = { id };
        continue;
      }
      // Still running past the Bash timeout: the skill does not load on a
      // half-run command, and the command does not outlive it.
      if (result.task || result.running) {
        abandon();
        return {
          failure:
            `Shell command failed for pattern "${raw}": [stderr]\nThe command did not complete within the Bash timeout.`,
        };
      }
      if (typeof result.deny === "string") {
        const output = result.deny.replace(/^Exit code \d+\n?/, "");
        return {
          failure: `Shell command failed for pattern "${raw}":${
            output ? ` [stderr]\n${output}` : ""
          }`,
        };
      }
      return {
        text: [result.result?.stdout, result.result?.stderr].filter(Boolean)
          .join("\n"),
      };
    }
  } finally {
    call.signal?.removeEventListener("abort", abandon);
  }
}

// A target skill's text with its marked shell commands run on the target.
export async function targetSkillShell(text, marker, run) {
  const fenced = new RegExp(
    "```" + marker + "!\\s*\\n?([\\s\\S]*?)\\n?```",
    "g",
  );
  const inline = new RegExp("(?<=^|\\s)!" + marker + "`([^`]+)`", "gm");
  const unmarked = (value) =>
    value.replaceAll("```" + marker + "!", "```!").replaceAll(
      "!" + marker + "`",
      "!`",
    );
  let result = text;
  for (const match of [...text.matchAll(fenced), ...text.matchAll(inline)]) {
    const command = match[1].trim();
    if (!command) continue;
    const outcome = await run(command, unmarked(match[0]));
    if (outcome.failure !== undefined) return outcome;
    result = result.replace(match[0], () => outcome.text);
  }
  return { text: unmarked(result) };
}

async function skillPrompt($, event, next) {
  const skills = context?.skills;
  const entry = typeof event.skill === "string" && skills &&
      event.skill.startsWith(skills.prefix)
    ? skills.entries.find((item) =>
      item.name === event.skill.slice(skills.prefix.length)
    )
    : undefined;
  if (!entry) return next(event);
  // A user's typed command has no Skill call; its expansion's own signal
  // cancels it.
  const call = skillCalls.get(event.skill) ?? { signal: next.signal };
  const expanded = await targetSkillShell(
    event.text,
    skills.marker,
    (command, raw) => skillShell($, command, raw, entry, call),
  );
  if (expanded.failure !== undefined) {
    call.failure = expanded.failure;
    return next({ ...event, text: expanded.failure });
  }
  return next({ ...event, text: targetSkillText(expanded.text, skills) });
}

// The target's MCP servers as launch.mjs describes them (mcp.mjs).
export function validMcp(mcp) {
  return Boolean(mcp) && Array.isArray(mcp.servers) &&
    mcp.servers.every((server) =>
      typeof server?.name === "string" &&
      server.prefix === "mcp__" + server.name + "__" &&
      ["target", "runtime"].includes(server.placement)
    ) && Array.isArray(mcp.omitted) &&
    mcp.omitted.every((server) =>
      typeof server?.name === "string" && typeof server.reason === "string"
    );
}

// The target's skills as launch.mjs describes them (skills.mjs).
export function validSkills(skills) {
  const strings = (value) =>
    Array.isArray(value) && value.every((item) => typeof item === "string");
  return Boolean(skills) && skills.prefix === "cowboy-target:" &&
    Array.isArray(skills.entries) &&
    (skills.entries.length === 0 ||
      /^cowboy[a-f0-9]{32}$/.test(skills.marker)) &&
    skills.entries.every((entry) =>
      typeof entry?.name === "string" && entry.name !== "" &&
      ["user", "project"].includes(entry.scope) &&
      strings(entry.allowedTools) &&
      (entry.kind === "command" ||
        (entry.kind === "skill" &&
          [entry.mirrorDirectory, entry.targetDirectory].every((path) =>
            typeof path === "string" && path.startsWith("/") &&
            !path.endsWith("/")
          )))
    ) &&
    Array.isArray(skills.omitted) &&
    skills.omitted.every((entry) =>
      typeof entry?.name === "string" && typeof entry.reason === "string"
    ) && strings(skills.bundled) &&
    Boolean(skills.unavailable) && typeof skills.unavailable === "object" &&
    Object.values(skills.unavailable).every((reason) =>
      typeof reason === "string"
    );
}

// Strings of a native message about the target's skills, renamed. A stored
// skill listing (within its reminder) is projected as it renders.
function targetSkillValue(value, skills) {
  if (typeof value === "string") {
    const listing =
      /^(<system-reminder>\n)?(The following skills are available for use with the Skill tool:[\s\S]*?)(\n<\/system-reminder>\n*)?$/
        .exec(value);
    return listing
      ? (listing[1] ?? "") + targetSkillListing(listing[2], skills) +
        (listing[3] ?? "")
      : targetSkillText(value, skills);
  }
  if (Array.isArray(value)) {
    return value.map((item) => targetSkillValue(item, skills));
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map((
        [key, item],
      ) => [key, targetSkillValue(item, skills)]),
    );
  }
  return value;
}

// The Skill tool: a target skill under its native name, a bundled one that
// works here, or a refusal that says why.
async function skillCall($, event, next) {
  const skills = context.skills;
  const requested = typeof event.skill === "string"
    ? event.skill.replace(/^\//, "")
    : event.skill;
  const target = skills.entries.some((entry) => entry.name === requested);
  if (!target && Object.hasOwn(skills.unavailable, requested)) {
    return {
      deny: `The ${requested} skill is unavailable in this execution session: ${
        skills.unavailable[requested]
      }.`,
    };
  }
  const omitted = skills.omitted.find((entry) => entry.name === requested);
  if (!target && omitted) {
    return {
      deny:
        `The ${requested} skill is unavailable in this execution session: ${omitted.reason}.`,
    };
  }
  const skill = target ? skills.prefix + requested : event.skill;
  const call = { owner: event.agentId, signal: next.signal };
  const previous = skillTurns.get(skill);
  const turn = Promise.withResolvers();
  const queued = (previous ?? Promise.resolve()).then(() => turn.promise);
  skillTurns.set(skill, queued);
  let result;
  try {
    await previous;
    if (
      call.signal?.aborted ||
      (call.owner !== undefined && stoppedAgents.has(call.owner))
    ) return { deny: "Tool call was cancelled" };
    skillCalls.set(skill, call);
    result = await next({ ...event, skill });
  } finally {
    if (skillCalls.get(skill) === call) skillCalls.delete(skill);
    turn.resolve();
    if (skillTurns.get(skill) === queued) skillTurns.delete(skill);
  }
  if (call.failure !== undefined) return { deny: call.failure };
  // Unchanged, native keeps the skill's own messages with its result; the
  // names in both are projected as they are appended (targetSkillAppend).
  return result;
}

// Attachments that name skills, projected as they are stored and rendered.
const SKILL_ATTACHMENTS = [
  "skill_listing",
  "invoked_skills",
  "command_permissions",
  "skill_mentions",
];

// The skill a typed command names, in native's command tags; the user's own
// arguments are left as typed.
function targetCommandNames(value, skills) {
  return typeof value === "string"
    ? value.replace(
      /<(command-message|command-name)>([^<]*)<\/\1>/g,
      (_tag, name, text) =>
        `<${name}>${targetSkillText(text, skills)}</${name}>`,
    )
    : Array.isArray(value)
    ? value.map((item) => targetCommandNames(item, skills))
    : typeof value?.text === "string"
    ? { ...value, text: targetCommandNames(value.text, skills) }
    : value;
}

// A message about the target's skills as it enters the session: the Skill
// tool's result and messages, a skill attachment, and the name of a command
// the user typed. Other messages (file contents, the user's own text) keep
// what they say.
export function targetSkillAppend(event, skills) {
  const message = event?.message;
  if (
    !skills?.entries.length ||
    !(Array.isArray(message?.content) || typeof message?.content === "string")
  ) return event;
  const project = (event.origin?.kind === "tool" &&
      event.origin.tool === "Skill" &&
      ["tool-result", "tool-message"].includes(event.door)) ||
      (event.door === "attachment" && SKILL_ATTACHMENTS.includes(message.name))
    ? targetSkillValue
    : event.door === "command"
    ? targetCommandNames
    : undefined;
  return project
    ? {
      ...event,
      message: { ...message, content: project(message.content, skills) },
    }
    : event;
}

// Native tools that run where the session runs (see launch.mjs).
const RUNTIME_TOOLS = [
  "AskUserQuestion",
  "TaskCreate",
  "TaskGet",
  "TaskList",
  "TaskUpdate",
  "WebFetch",
  "WebSearch",
  "ReportFindings",
];

// A URL naming the machine itself means the target's in a local session.
export function targetLoopback(url) {
  let host;
  try {
    host = new URL(String(url)).hostname.toLowerCase().replace(/^\[|\]$/g, "");
  } catch {
    return false;
  }
  // A trailing dot names the same host; an IPv4-mapped address the same IP.
  host = host.replace(/\.$/, "").replace(/^::ffff:/, "");
  return host === "localhost" || host.endsWith(".localhost") ||
    host === "::1" || host === "0.0.0.0" || host === "::" ||
    /^127\.\d+\.\d+\.\d+$/.test(host) ||
    /^7f[0-9a-f]{2}:[0-9a-f]{1,4}$/.test(host);
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
  // Target tools keep native's own descriptions, as in a local session.
  on("tool.describe", (_$, event, next) => next(event));

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
    if (event.tool === "WebFetch" && targetLoopback(event.url)) {
      return {
        deny:
          "WebFetch runs where this session runs, not on the target machine, so it cannot reach the target's localhost. Fetch the URL on the target with Bash (for example curl) instead.",
      };
    }
    if (RUNTIME_TOOLS.includes(event.tool)) return next(event);
    // The target's MCP servers: native owns the call; a stdio server's
    // process runs on the target behind its relay (mcp.mjs).
    if (
      typeof event.tool === "string" &&
      context?.mcp.servers.some((server) =>
        event.tool.startsWith(server.prefix)
      )
    ) return next(event);
    if (event.tool === "Skill") {
      return context ? skillCall($, event, next) : { deny: unavailable };
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
        if (response.status !== 202 && typeof result.localRead === "string") {
          result = await nativeLocalRead($, event, next, result);
        }
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
          const { task, instructions: nested, running: _running, ...answered } =
            result;
          if (task) {
            answered.result = await notifyOnEnd(
              $,
              event,
              task,
              answered.result,
              input,
              next,
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

  // Attachments about this machine's files are left out; those naming the
  // target's skills use their native names.
  on(
    "prompt.attachment",
    (_$, event, next) =>
      RUNTIME_ATTACHMENTS.has(event.type) ? { text: null } : next({
        ...event,
        text: event.type === "skill_listing"
          ? targetSkillListing(event.text, context?.skills)
          : SKILL_ATTACHMENTS.includes(event.type)
          ? targetSkillText(event.text, context?.skills)
          : event.text,
      }),
  ).catch(() => ({ text: null }));
  // A target skill whose commands could not run is not shown half-expanded.
  on("skill.prompt", skillPrompt).catch(() => ({
    text:
      "This skill could not be loaded from the target. Inspect the target before retrying it.",
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
    (_$, event, next) => next(targetSkillAppend(event, context?.skills)),
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
    async ($, event, next) => {
      const unconfirmed = await stopAtDeadline($, event);
      return next(
        unconfirmedStop(
          targetTaskNotification(event, agentOutputs),
          unconfirmed,
        ),
      );
    },
  );
  on(
    "session.append",
    { door: "delivery" },
    async ($, event, next) => {
      const unconfirmed = await stopAtDeadline($, event);
      return next(
        unconfirmedStop(
          targetTaskNotification(event, agentOutputs),
          unconfirmed,
        ),
      );
    },
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
      ) || !validSkills(loaded.skills) || !validMcp(loaded.mcp)
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
