// Mode classification, mode generations, and attempt-scoped UI call correlation
// for Antigravity.
//
// In restricted modes (`plan`, `"switching"`, `undefined`, or any unrecognized
// mode string), Cowboy enforces mode policy directly inside `Facade.answerOnce`
// before any tool handler, extension RPC, or inflight tracking runs:
//   - reads, `UpdatePlan`, `StopCommand` and `StopSubagent` run without asking;
//   - `WriteFile`, `EditFile` and `Subagent` are refused unconditionally;
//   - `RunCommand` and extension tools request single-use owner approval
//     directly from `Facade.answerOnce` using the exact frozen execution
//     payload and a proxy-allocated incarnation-scoped JSON-RPC ID bound 1-to-1
//     to that single invocation's Promise.
//
// Only explicitly confirmed unrestricted modes (`default`, `accept-edits`,
// `auto`) admit writes, subagents, and unprompted commands/extensions.
import { createHash } from "node:crypto";

export const FACADE_PREFIX = "mcp_cowboy_";

/** Confirmed native modes that allow unrestricted Cowboy tool dispatch. */
export const UNRESTRICTED_MODES = Object.freeze(new Set(["default", "accept-edits", "auto"]));

/** Facade tools allowed without owner approval in restricted/plan modes. */
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

/** Facade tools refused unconditionally in restricted/plan modes. */
export const PLAN_REFUSED_TOOLS = Object.freeze(new Set(["WriteFile", "EditFile", "Subagent"]));

/** Status values after which an ACP tool call emits nothing more. */
const TERMINAL = new Set(["completed", "failed"]);
/** Bounded active announcements tracked per session for UI presentation. */
export const MAX_WAITING = 64;

/** Strip `mcp_cowboy_` from an ACP tool name, or `undefined`. */
export function stripFacadePrefix(name) {
  if (typeof name !== "string" || !name.startsWith(FACADE_PREFIX)) return undefined;
  const bare = name.slice(FACADE_PREFIX.length);
  return bare.length > 0 ? bare : undefined;
}

/** Plan-mode policy for one facade or extension tool name. */
export function planToolPolicy(toolName) {
  if (typeof toolName !== "string" || toolName.length === 0) return "refuse";
  if (PLAN_ALLOWED_TOOLS.has(toolName)) return "allow";
  if (PLAN_REFUSED_TOOLS.has(toolName)) return "refuse";
  return "approve";
}

/** Whether `mode` is an explicitly confirmed unrestricted mode. */
export function isUnrestrictedMode(mode) {
  return typeof mode === "string" && UNRESTRICTED_MODES.has(mode);
}

/**
 * Whether `mode` fails closed into restricted/plan enforcement: true for
 * `"plan"`, `"switching"`, `undefined`, and any unrecognized mode string.
 */
export function isPlanEffectiveMode(mode) {
  return !isUnrestrictedMode(mode);
}

function isBuiltinFacadeTool(toolName) {
  return (
    PLAN_ALLOWED_TOOLS.has(toolName) ||
    PLAN_REFUSED_TOOLS.has(toolName) ||
    toolName === "RunCommand"
  );
}

/** Deterministic SHA-256 key of a tool call's arguments for UI card correlation. */
export function argsDigest(input, { stripSummary = true } = {}) {
  if (input === null || typeof input !== "object" || Array.isArray(input)) return undefined;
  const target = stripSummary ? (({ toolSummary: _s, ...rest }) => rest)(input) : input;
  try {
    const serialized = JSON.stringify(target, (_key, value) => {
      if (typeof value === "number" && !Number.isFinite(value)) {
        throw new Error("non-finite number");
      }
      if (value && typeof value === "object" && !Array.isArray(value)) {
        const sorted = {};
        for (const key of Object.keys(value).sort()) {
          if (value[key] !== undefined) sorted[key] = value[key];
        }
        return sorted;
      }
      return value;
    });
    if (typeof serialized !== "string") return undefined;
    return createHash("sha256").update(serialized).digest("hex");
  } catch {
    return undefined;
  }
}

function isTurnLive(turn) {
  return turn !== undefined && !turn.cancelled && !turn.closed;
}

function attemptOf(turn) {
  return turn?.attempt;
}

/**
 * Mode generations and attempt-scoped UI call correlation for one `AcpProxy`.
 */
export class PlanGate {
  constructor() {
    /** Monotonic generation per root owner session, bumped on mode changes/switches. */
    this.modeGens = new Map();
    /** Announced active ACP `cowboy` calls per session: sessionId -> Array<Entry>. */
    this.waiting = new Map();
    /** Facade call object -> `{ turn, attempt }` captured at admission. */
    this.callAttempts = new WeakMap();
    /** Facade call object -> `{ entry, id, turn, attempt, digest }` bound for UI callbacks. */
    this.bound = new WeakMap();
  }

  /** Current mode generation for `ownerId`. */
  modeGeneration(ownerId) {
    return this.modeGens.get(ownerId) ?? 0;
  }

  /** Advance `ownerId`'s mode generation (called only for root owner sessions). */
  bumpModeGeneration(ownerId) {
    if (typeof ownerId !== "string") return 0;
    const next = this.modeGeneration(ownerId) + 1;
    this.modeGens.set(ownerId, next);
    return next;
  }

  /** Clear all tracked UI announcements (e.g. on proxy shutdown). */
  clearAll() {
    this.waiting.clear();
  }

  /**
   * Clear waiting announcements belonging to `turn` (or all announcements for
   * `sessionId` when `turn` is `undefined`), returning unresolved `toolCallId`s.
   */
  clearTurn(sessionId, turn) {
    if (typeof sessionId !== "string") return [];
    const list = this.waiting.get(sessionId);
    if (!list) return [];
    if (turn === undefined) {
      this.waiting.delete(sessionId);
      return list.map((entry) => entry.id);
    }
    const removed = [];
    const left = [];
    for (const entry of list) {
      if (entry.turn === turn) removed.push(entry.id);
      else left.push(entry);
    }
    if (left.length) this.waiting.set(sessionId, left);
    else this.waiting.delete(sessionId);
    return removed;
  }

  /**
   * Record one `tool_call` or `tool_call_update` emitted by `agy` for UI
   * progress/result attachment. Validates `name` and `rawInput` against an
   * existing entry before removing terminal entries, returning `{ contradicted,
   * contradictedIds, evictedId }` so the proxy can drop stale buffered results.
   */
  trackUpdate(sessionId, update, turn) {
    const id = update?.toolCallId;
    if (typeof sessionId !== "string" || typeof id !== "string" || id === "") {
      return { contradicted: false };
    }
    const list = this.waiting.get(sessionId) ?? [];
    const attempt = attemptOf(turn);
    const incomingName = typeof update.name === "string" ? update.name : undefined;
    const existing = list.find((entry) => entry.id === id);
    let contradicted = false;
    if (existing) {
      if (
        existing.turn !== turn ||
        existing.attempt !== attempt ||
        (incomingName !== undefined && incomingName !== existing.name)
      ) {
        existing.ambiguous = true;
        contradicted = true;
      }
      if (update.rawInput !== undefined) {
        const digest = argsDigest(update.rawInput, {
          stripSummary: isBuiltinFacadeTool(existing.toolName),
        });
        if (
          digest === undefined ||
          (existing.hasInput && existing.digest !== digest) ||
          (existing.boundDigest !== undefined && existing.boundDigest !== digest)
        ) {
          existing.ambiguous = true;
          contradicted = true;
        } else if (!existing.hasInput) {
          existing.hasInput = true;
          existing.digest = digest;
        }
      }
      if (existing.ambiguous && existing.boundCall) {
        this.bound.delete(existing.boundCall);
        existing.boundCall = undefined;
      }
    }
    if (TERMINAL.has(update.status)) {
      const left = list.filter((entry) => entry.id !== id);
      if (left.length) this.waiting.set(sessionId, left);
      else this.waiting.delete(sessionId);
      return contradicted ? { contradicted: true, contradictedIds: [id] } : { contradicted: false };
    }
    if (existing) {
      return contradicted ? { contradicted: true, contradictedIds: [id] } : { contradicted: false };
    }
    const bare = stripFacadePrefix(update.name);
    if (bare === undefined || !isTurnLive(turn)) {
      return { contradicted: false };
    }
    let evictedId;
    if (list.length >= MAX_WAITING) {
      const evicted = list.shift();
      evictedId = evicted?.id;
      if (evicted?.boundCall) {
        this.bound.delete(evicted.boundCall);
        evicted.boundCall = undefined;
      }
    }
    const hasRaw = update.rawInput !== undefined;
    const digest = hasRaw
      ? argsDigest(update.rawInput, { stripSummary: isBuiltinFacadeTool(bare) })
      : undefined;
    list.push({
      id,
      name: update.name,
      toolName: bare,
      turn,
      attempt,
      hasInput: hasRaw && digest !== undefined,
      digest,
      boundCall: undefined,
      boundDigest: undefined,
      ambiguous: hasRaw && digest === undefined,
    });
    this.waiting.set(sessionId, list);
    return { contradicted: false, evictedId };
  }

  /** Capture `call`'s turn and attempt lifetime at `Facade` admission. */
  admitCall(call, turn) {
    if (!call || typeof call !== "object") return;
    const attempt = attemptOf(turn);
    this.callAttempts.set(call, { turn, attempt });
    if (turn !== undefined && !isTurnLive(turn)) return;
    this.matchUnbound(call, turn, attempt);
  }

  matchUnbound(call, turn, attempt) {
    const list = this.waiting.get(call.sessionId) ?? [];
    const name = `${FACADE_PREFIX}${call.name}`;
    const digest = argsDigest(call.args === undefined ? {} : call.args, {
      stripSummary: isBuiltinFacadeTool(call.name),
    });
    const isDiffBearing = call.name === "WriteFile" || call.name === "EditFile";
    const sameTool = list.filter(
      (item) => item.name === name && item.turn === turn && item.attempt === attempt,
    );
    const unbound = sameTool.filter((item) => !item.boundCall && !item.ambiguous);
    let entry;
    if (digest !== undefined) {
      const exactUnbound = unbound.filter((item) => item.hasInput && item.digest === digest);
      if (exactUnbound.length === 1) {
        entry = exactUnbound[0];
      } else if (exactUnbound.length > 1) {
        if (!isDiffBearing) entry = exactUnbound[0];
      } else {
        const inputless = unbound.filter((item) => !item.hasInput);
        if (inputless.length === 1 && (!isDiffBearing || sameTool.length === 1)) {
          entry = inputless[0];
        }
      }
    } else if (unbound.length === 1 && (!isDiffBearing || sameTool.length === 1)) {
      entry = unbound[0];
    }
    if (!entry) return undefined;
    entry.boundCall = call;
    entry.boundDigest = digest;
    this.bound.set(call, { entry, id: entry.id, turn, attempt, digest });
    return entry.id;
  }

  /** Return the ACP `toolCallId` bound to `call` for UI progress/result attachment. */
  bind(call, currentTurn) {
    if (!call || typeof call !== "object") return undefined;
    const currentAttempt = attemptOf(currentTurn);
    if (this.callAttempts.has(call)) {
      const admitted = this.callAttempts.get(call);
      if (admitted.turn !== currentTurn || admitted.attempt !== currentAttempt) return undefined;
      if (admitted.turn !== undefined && !isTurnLive(admitted.turn)) return undefined;
    } else if (currentTurn !== undefined && !isTurnLive(currentTurn)) {
      return undefined;
    }
    const binding = this.bound.get(call);
    if (binding) {
      if (binding.turn !== currentTurn || binding.attempt !== currentAttempt) return undefined;
      if (binding.turn !== undefined && !isTurnLive(binding.turn)) return undefined;
      if (binding.entry.ambiguous) return undefined;
      if (
        binding.digest !== undefined &&
        binding.entry.hasInput &&
        binding.entry.digest !== binding.digest
      ) {
        return undefined;
      }
      return binding.id;
    }
    return this.matchUnbound(call, currentTurn, currentAttempt);
  }

  /** Whether `call`'s previously bound UI announcement is still consistent and live. */
  isBindingConsistent(call, currentTurn) {
    if (!call || typeof call !== "object") return false;
    const binding = this.bound.get(call);
    if (!binding) return false;
    const currentAttempt = attemptOf(currentTurn);
    if (binding.turn !== currentTurn || binding.attempt !== currentAttempt) return false;
    if (binding.turn !== undefined && !isTurnLive(binding.turn)) return false;
    if (binding.entry.ambiguous) return false;
    if (
      binding.digest !== undefined &&
      binding.entry.hasInput &&
      binding.entry.digest !== binding.digest
    ) {
      return false;
    }
    return true;
  }
}
