// Call correlation and plan-mode dispatch enforcement for Antigravity (R3).
//
// `agy` announces every MCP tool call on ACP stdout (`tool_call`, with its
// `toolCallId` and arguments) before calling the local Cowboy MCP server, whose
// JSON-RPC request carries no ACP `toolCallId`. In plan mode (or when a
// session's mode is unknown or mid-switch), Cowboy enforces the mode directly
// at facade dispatch:
//   - reads, `UpdatePlan`, `StopCommand` and `StopSubagent` run without asking;
//   - `WriteFile`, `EditFile` and `Subagent` are refused unconditionally;
//   - `RunCommand` and extension tools run only with a single-use owner
//     approval bound to the exact native session, active turn attempt, tool
//     name, canonical arguments and mode generation, and only when the
//     announced ACP call correlates unambiguously (1-to-1) at dispatch.
//
// Any ambiguity (concurrent identical calls, unannounced call, missing/mutated
// or oversized arguments, waiting-table overflow) fails closed. Grants expire
// on consumption, terminal call update, turn cancel, turn end, or mode change.

export const FACADE_PREFIX = "mcp_cowboy_";

/** Facade tools allowed without owner approval in plan / unknown / switching mode. */
export const PLAN_ALLOWED_TOOLS = Object.freeze(
  new Set([
    "ReadFile",
    "ListDirectory",
    "SearchFiles",
    "UpdatePlan",
    "StopCommand",
    "StopSubagent",
  ]),
);

/** Facade tools refused unconditionally in plan / unknown / switching mode. */
export const PLAN_REFUSED_TOOLS = Object.freeze(new Set(["WriteFile", "EditFile", "Subagent"]));

/** Status values after which an ACP tool call emits nothing more. */
const TERMINAL = new Set(["completed", "failed"]);
/** Bounded active announcements tracked per session. */
export const MAX_WAITING = 64;
/** Bounded active single-use plan grants tracked per session. */
export const MAX_PLAN_GRANTS = 128;
const MAX_CANONICAL_DEPTH = 32;
const MAX_CANONICAL_BYTES = 128 * 1024;

/** Strip `mcp_cowboy_` from an ACP tool name, or `undefined`. */
export function stripFacadePrefix(name) {
  if (typeof name !== "string" || !name.startsWith(FACADE_PREFIX)) return undefined;
  const bare = name.slice(FACADE_PREFIX.length);
  return bare.length > 0 ? bare : undefined;
}

/** Plan-mode policy for one facade or extension tool name. */
export function planToolPolicy(toolName) {
  if (PLAN_ALLOWED_TOOLS.has(toolName)) return "allow";
  if (PLAN_REFUSED_TOOLS.has(toolName)) return "refuse";
  return "approve";
}

/** Whether `mode` counts as plan mode (unknown or switching fails closed). */
export function isPlanEffectiveMode(mode) {
  return mode === undefined || mode === "switching" || mode === "plan";
}

/** Recursively canonicalize one JSON value; returns `undefined` on invalid/deep values. */
function canonicalValue(value, depth) {
  if (depth > MAX_CANONICAL_DEPTH) return undefined;
  if (value === null || typeof value === "boolean" || typeof value === "string") {
    return JSON.stringify(value);
  }
  if (typeof value === "number") {
    return Number.isFinite(value) ? JSON.stringify(value) : undefined;
  }
  if (Array.isArray(value)) {
    const parts = [];
    for (const item of value) {
      const encoded = item === undefined ? "null" : canonicalValue(item, depth + 1);
      if (encoded === undefined) return undefined;
      parts.push(encoded);
    }
    return `[${parts.join(",")}]`;
  }
  if (typeof value === "object") {
    const keys = Object.keys(value)
      .filter((key) => value[key] !== undefined)
      .sort();
    const parts = [];
    for (const key of keys) {
      const encoded = canonicalValue(value[key], depth + 1);
      if (encoded === undefined) return undefined;
      parts.push(`${JSON.stringify(key)}:${encoded}`);
    }
    return `{${parts.join(",")}}`;
  }
  return undefined;
}

/**
 * Canonical argument string for one tool call, with top-level `toolSummary`
 * dropped and all nested object keys sorted deterministically. Returns
 * `undefined` when `input` is not a plain object or exceeds safety bounds.
 */
export function canonicalArgsKey(input) {
  if (input === null || typeof input !== "object" || Array.isArray(input)) return undefined;
  const { toolSummary: _summary, ...rest } = input;
  const encoded = canonicalValue(rest, 0);
  if (encoded === undefined || Buffer.byteLength(encoded) > MAX_CANONICAL_BYTES) {
    return undefined;
  }
  return encoded;
}

/**
 * Presentation-safe argument key (non-strict fallback for UI correlation when
 * a legacy mock omits `rawInput`).
 */
export function presentationArgsKey(input) {
  return canonicalArgsKey(input) ?? "{}";
}

/**
 * Shared call-correlation and plan-mode approval state for one `AcpProxy`.
 */
export class PlanGate {
  constructor() {
    /** Monotonic generation per owner (root) session, bumped on every mode change or switch. */
    this.modeGens = new Map();
    /** Active announced `mcp_cowboy_*` calls per session. */
    this.waiting = new Map();
    /** Facade call object -> correlated ACP `toolCallId`. */
    this.bound = new WeakMap();
    /** Unused single-use plan grants: sessionId -> Map<toolCallId, Grant>. */
    this.grants = new Map();
    /** Turns whose `waiting` queue overflowed (fail-closed for plan approvals). */
    this.overflowTurns = new WeakSet();
    /** Refused ambiguous facade calls that bind() must not heuristically attach. */
    this.uncorrelatable = new WeakSet();
  }

  /** Clear all tracked announcements and unused grants (e.g. on proxy shutdown). */
  clearAll() {
    this.waiting.clear();
    this.grants.clear();
  }

  /** Current mode generation for `ownerId`. */
  modeGeneration(ownerId) {
    return this.modeGens.get(ownerId) ?? 0;
  }

  /**
   * Bump `ownerId`'s mode generation and clear every unused plan grant on
   * `ownerId` and all its active subagents.
   */
  invalidateOwner(ownerId, childSessionIds = []) {
    if (typeof ownerId !== "string") return;
    this.modeGens.set(ownerId, this.modeGeneration(ownerId) + 1);
    this.grants.delete(ownerId);
    for (const childId of childSessionIds) {
      if (typeof childId === "string") this.grants.delete(childId);
    }
  }

  /**
   * Clear grants and waiting announcements belonging to `turn` (or every entry
   * of `sessionId` when `turn` is `undefined`).
   */
  clearTurn(sessionId, turn) {
    if (typeof sessionId !== "string") return;
    const grants = this.grants.get(sessionId);
    if (grants) {
      if (turn === undefined) {
        this.grants.delete(sessionId);
      } else {
        for (const [id, grant] of grants) {
          if (grant.turn === turn) grants.delete(id);
        }
        if (grants.size === 0) this.grants.delete(sessionId);
      }
    }
    const list = this.waiting.get(sessionId);
    if (list) {
      if (turn === undefined) {
        this.waiting.delete(sessionId);
      } else {
        const left = list.filter((entry) => entry.turn !== turn);
        if (left.length) this.waiting.set(sessionId, left);
        else this.waiting.delete(sessionId);
      }
    }
  }

  /** Revoke any unused grant for `(sessionId, toolCallId)`. */
  revokeCall(sessionId, toolCallId) {
    const grants = this.grants.get(sessionId);
    if (!grants) return;
    grants.delete(toolCallId);
    if (grants.size === 0) this.grants.delete(sessionId);
  }

  /** Push one new waiting entry, latching overflow on affected turns if bounded eviction occurs. */
  pushWaiting(sessionId, entry, turn) {
    const list = this.waiting.get(sessionId) ?? [];
    if (list.length >= MAX_WAITING) {
      const dropped = list.shift();
      if (dropped?.turn) this.overflowTurns.add(dropped.turn);
      if (turn) this.overflowTurns.add(turn);
      if (dropped) this.revokeCall(sessionId, dropped.id);
    }
    list.push(entry);
    this.waiting.set(sessionId, list);
    return entry;
  }

  /**
   * Record one `tool_call` or `tool_call_update` emitted by `agy`.
   * Terminal updates remove the entry and revoke any unused plan grant.
   */
  trackUpdate(sessionId, update, turn) {
    const id = update?.toolCallId;
    if (typeof sessionId !== "string" || typeof id !== "string" || id === "") return;
    const list = this.waiting.get(sessionId) ?? [];
    if (TERMINAL.has(update.status)) {
      const left = list.filter((entry) => entry.id !== id);
      if (left.length) this.waiting.set(sessionId, left);
      else this.waiting.delete(sessionId);
      this.revokeCall(sessionId, id);
      return;
    }
    const existing = list.find((entry) => entry.id === id);
    if (existing) {
      if (existing.turn !== turn) existing.ambiguous = true;
      if (typeof update.name === "string" && update.name !== existing.name) {
        existing.ambiguous = true;
      }
      if (update.rawInput !== undefined) {
        const key = canonicalArgsKey(update.rawInput);
        if (key === undefined) existing.ambiguous = true;
        else if (!existing.hasInput) {
          existing.hasInput = true;
          existing.key = key;
        } else if (existing.key !== key) {
          existing.ambiguous = true;
        }
      }
      if (existing.ambiguous) this.revokeCall(sessionId, id);
      return;
    }
    const name = update.name;
    const toolName = stripFacadePrefix(name);
    if (toolName === undefined) return;
    const hasRaw = update.rawInput !== undefined;
    const key = hasRaw ? canonicalArgsKey(update.rawInput) : undefined;
    this.pushWaiting(
      sessionId,
      {
        id,
        name,
        toolName,
        turn,
        hasInput: hasRaw && key !== undefined,
        key,
        ambiguous: hasRaw && key === undefined,
        dispatched: false,
        call: undefined,
      },
      turn,
    );
  }

  /**
   * Validate and bind one `session/request_permission` for an approval-gated
   * tool (`RunCommand` or extension) in plan / unknown / switching mode.
   * Fails closed when there is no active turn, no `toolCallId`, missing or
   * invalid/mismatched arguments, or waiting-table overflow.
   */
  preparePermission(sessionId, ownerId, toolCall, turn) {
    if (typeof sessionId !== "string" || typeof ownerId !== "string") {
      return { ok: false, reason: "missing session" };
    }
    if (!turn || turn.cancelled || this.overflowTurns.has(turn)) {
      return { ok: false, reason: "no correlatable active turn" };
    }
    const id = toolCall?.toolCallId;
    const name = toolCall?.name;
    const toolName = stripFacadePrefix(name);
    if (typeof id !== "string" || id === "" || toolName === undefined) {
      return { ok: false, reason: "missing toolCallId or tool name" };
    }
    const list = this.waiting.get(sessionId) ?? [];
    let entry = list.find((item) => item.id === id);
    const hasRaw = toolCall?.rawInput !== undefined;
    const reqKey = hasRaw ? canonicalArgsKey(toolCall.rawInput) : undefined;
    if (hasRaw && reqKey === undefined) {
      if (entry) entry.ambiguous = true;
      return { ok: false, reason: "invalid or oversized tool arguments" };
    }
    if (entry) {
      if (entry.turn !== turn || entry.name !== name || entry.dispatched) {
        entry.ambiguous = true;
        return { ok: false, reason: "toolCallId conflicts with an earlier call" };
      }
      if (hasRaw) {
        if (entry.hasInput && entry.key !== reqKey) {
          entry.ambiguous = true;
          return { ok: false, reason: "permission arguments differ from announced call" };
        }
        entry.hasInput = true;
        entry.key = reqKey;
      }
      if (entry.ambiguous || !entry.hasInput || entry.key === undefined) {
        return { ok: false, reason: "announced call has no canonical arguments" };
      }
    } else {
      if (!hasRaw || reqKey === undefined) {
        return { ok: false, reason: "permission request has no canonical arguments" };
      }
      entry = this.pushWaiting(
        sessionId,
        {
          id,
          name,
          toolName,
          turn,
          hasInput: true,
          key: reqKey,
          ambiguous: false,
          dispatched: false,
          call: undefined,
        },
        turn,
      );
    }
    return {
      ok: true,
      token: {
        sessionId,
        ownerId,
        turn,
        modeGen: this.modeGeneration(ownerId),
        toolCallId: entry.id,
        toolName: entry.toolName,
        argsKey: entry.key,
      },
    };
  }

  /**
   * Process the owner's reply to a plan-mode permission request. Mints a
   * single-use grant only when `allowed === true` and the session's active turn,
   * mode generation, and announced call are still valid and unchanged.
   *
   * Returns `{ granted, forwardCancelled }`. When `forwardCancelled` is true,
   * the caller must reply `cancelled` to `agy` instead of forwarding a stale
   * approval.
   */
  recordPermissionReply(token, allowed, currentTurn) {
    if (!token) return { granted: false, forwardCancelled: false };
    const turnValid =
      currentTurn !== undefined && currentTurn === token.turn && !currentTurn.cancelled;
    const modeValid = this.modeGeneration(token.ownerId) === token.modeGen;
    const list = this.waiting.get(token.sessionId) ?? [];
    const entry = list.find((item) => item.id === token.toolCallId);
    const entryValid =
      entry !== undefined &&
      entry.turn === token.turn &&
      !entry.ambiguous &&
      !entry.dispatched &&
      entry.hasInput &&
      entry.key === token.argsKey;
    if (!turnValid || !modeValid || !entryValid) {
      return {
        granted: false,
        forwardCancelled: allowed,
      };
    }
    if (!allowed) return { granted: false, forwardCancelled: false };
    let sessionGrants = this.grants.get(token.sessionId);
    if (!sessionGrants) {
      sessionGrants = new Map();
      this.grants.set(token.sessionId, sessionGrants);
    }
    if (sessionGrants.size >= MAX_PLAN_GRANTS && !sessionGrants.has(token.toolCallId)) {
      return { granted: false, forwardCancelled: true };
    }
    sessionGrants.set(token.toolCallId, {
      toolCallId: token.toolCallId,
      toolName: token.toolName,
      argsKey: token.argsKey,
      turn: token.turn,
      ownerId: token.ownerId,
      modeGen: token.modeGen,
      used: false,
    });
    return { granted: true, forwardCancelled: false };
  }

  /**
   * Enforce plan mode synchronously at facade dispatch. Returns
   * `{ allowed: true, toolCallId? }` or `{ allowed: false, reason }`.
   */
  authorizeDispatch(call, { mode, turn, ownerId, managed, rawArgs } = {}) {
    if (managed === true) return { allowed: true };
    if (!isPlanEffectiveMode(mode)) return { allowed: true };
    const name = call?.name;
    const policy = planToolPolicy(name);
    if (policy === "allow") return { allowed: true };
    if (policy === "refuse") {
      const reason =
        mode === "plan"
          ? `${name} is not available in plan mode.`
          : `${name} is unavailable while the session's mode is ${mode ?? "unknown"}.`;
      return { allowed: false, reason };
    }
    // `RunCommand` and extension calls require an unambiguous single-use grant.
    if (!turn || turn.cancelled) {
      return { allowed: false, reason: `No active turn to authorize ${name} in plan mode.` };
    }
    if (this.overflowTurns.has(turn)) {
      if (call && typeof call === "object") this.uncorrelatable.add(call);
      return {
        allowed: false,
        reason: `${name} refused in plan mode: too many concurrent calls to correlate approval unambiguously.`,
      };
    }
    const input = rawArgs !== undefined ? rawArgs : call?.args === undefined ? {} : call.args;
    const argsKey = canonicalArgsKey(input);
    if (argsKey === undefined) {
      if (call && typeof call === "object") this.uncorrelatable.add(call);
      return {
        allowed: false,
        reason: `${name} refused in plan mode: arguments cannot be correlated unambiguously.`,
      };
    }
    const list = this.waiting.get(call.sessionId) ?? [];
    // Include any undispatched call of the same tool whose arguments are missing
    // or ambiguous, because it could collide with this MCP invocation.
    const candidates = list.filter(
      (entry) =>
        entry.turn === turn &&
        entry.toolName === name &&
        !entry.dispatched &&
        (entry.ambiguous || !entry.hasInput || entry.key === argsKey),
    );
    if (candidates.length === 0) {
      const reason =
        mode === "plan"
          ? `${name} requires explicit owner approval in plan mode.`
          : `${name} requires a confirmed non-plan mode or explicit owner approval.`;
      return { allowed: false, reason };
    }
    if (candidates.length > 1) {
      if (call && typeof call === "object") this.uncorrelatable.add(call);
      return {
        allowed: false,
        reason: `${name} refused in plan mode: ambiguous concurrent calls cannot be correlated unambiguously.`,
      };
    }
    const [entry] = candidates;
    if (entry.ambiguous || !entry.hasInput || entry.key !== argsKey) {
      return {
        allowed: false,
        reason: `${name} refused in plan mode: call announcement cannot be correlated unambiguously.`,
      };
    }
    const sessionGrants = this.grants.get(call.sessionId);
    const grant = sessionGrants?.get(entry.id);
    const currentGen = this.modeGeneration(ownerId);
    if (
      !grant ||
      grant.used ||
      grant.turn !== turn ||
      grant.toolName !== name ||
      grant.argsKey !== argsKey ||
      grant.ownerId !== ownerId ||
      grant.modeGen !== currentGen
    ) {
      const reason =
        mode === "plan"
          ? `${name} requires explicit owner approval in plan mode.`
          : `${name} requires a confirmed non-plan mode or explicit owner approval.`;
      return { allowed: false, reason };
    }
    // Consume the grant synchronously before any async tool work begins.
    grant.used = true;
    sessionGrants.delete(entry.id);
    if (sessionGrants.size === 0) this.grants.delete(call.sessionId);
    entry.dispatched = true;
    entry.call = call;
    this.bound.set(call, entry.id);
    return { allowed: true, toolCallId: entry.id };
  }

  /**
   * The ACP `toolCallId` one facade `call` answers for UI presentation
   * (`toolProgress`, `toolResult`). Returns the exact dispatch-bound ID when
   * present, else falls back to the oldest unbound matching announcement.
   */
  bind(call) {
    if (this.bound.has(call)) return this.bound.get(call);
    if (call && typeof call === "object" && this.uncorrelatable.has(call)) return undefined;
    const list = this.waiting.get(call.sessionId) ?? [];
    const name = `${FACADE_PREFIX}${call.name}`;
    const key = presentationArgsKey(call.args);
    const entry =
      list.find((item) => !item.call && item.name === name && (item.key ?? "{}") === key) ??
      list.find((item) => !item.call && item.name === name);
    if (!entry) return undefined;
    entry.call = call;
    this.bound.set(call, entry.id);
    return entry.id;
  }
}
