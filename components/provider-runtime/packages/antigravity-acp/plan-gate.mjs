// Mode classification, mode generations, and turn-scoped UI call correlation
// for Antigravity (R3).
//
// In restricted modes (`plan`, `"switching"`, `undefined`, or any unrecognized
// mode string), Cowboy enforces mode policy directly inside `Facade.answerOnce`
// before any tool handler or extension RPC runs:
//   - reads, `UpdatePlan`, `StopCommand` and `StopSubagent` run without asking;
//   - `WriteFile`, `EditFile` and `Subagent` are refused unconditionally;
//   - `RunCommand` and extension tools request single-use owner approval
//     directly from `Facade.answerOnce` using the exact execution payload and a
//     proxy-allocated JSON-RPC ID bound to that single invocation's Promise.
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
const MAX_DIGEST_DEPTH = 64;

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

/** Recursively feed canonical JSON tokens into `hash`; returns false on invalid/cyclic input. */
function hashCanonical(hash, value, depth, seen) {
  if (depth > MAX_DIGEST_DEPTH) return false;
  if (value === null || typeof value === "boolean" || typeof value === "string") {
    hash.update(JSON.stringify(value));
    return true;
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) return false;
    hash.update(JSON.stringify(value));
    return true;
  }
  if (Array.isArray(value)) {
    if (seen.has(value)) return false;
    seen.add(value);
    hash.update("[");
    for (let i = 0; i < value.length; i++) {
      if (i > 0) hash.update(",");
      const item = value[i];
      if (item === undefined) hash.update("null");
      else if (!hashCanonical(hash, item, depth + 1, seen)) {
        seen.delete(value);
        return false;
      }
    }
    hash.update("]");
    seen.delete(value);
    return true;
  }
  if (typeof value === "object") {
    if (seen.has(value)) return false;
    seen.add(value);
    const keys = Object.keys(value)
      .filter((key) => value[key] !== undefined)
      .sort();
    hash.update("{");
    for (let i = 0; i < keys.length; i++) {
      if (i > 0) hash.update(",");
      const key = keys[i];
      hash.update(JSON.stringify(key));
      hash.update(":");
      if (!hashCanonical(hash, value[key], depth + 1, seen)) {
        seen.delete(value);
        return false;
      }
    }
    hash.update("}");
    seen.delete(value);
    return true;
  }
  return false;
}

/**
 * Deterministic SHA-256 digest of a tool call's arguments for UI correlation,
 * supporting arbitrarily large payloads (e.g. > 128 KiB file writes) without
 * collapsing distinct payloads together. Strips top-level `toolSummary` only
 * when `stripSummary` is true (built-in facade tools).
 */
export function argsDigest(input, { stripSummary = true } = {}) {
  if (input === null || typeof input !== "object" || Array.isArray(input)) return undefined;
  const target = stripSummary ? (({ toolSummary: _s, ...rest }) => rest)(input) : input;
  const hash = createHash("sha256");
  if (!hashCanonical(hash, target, 0, new Set())) return undefined;
  return hash.digest("hex");
}

/** Whether a turn record is still open for binding UI progress/results. */
function isTurnLive(turn) {
  return turn !== undefined && !turn.cancelled && !turn.closed;
}

/**
 * Mode generations and turn-scoped UI call correlation for one `AcpProxy`.
 */
export class PlanGate {
  constructor() {
    /** Monotonic generation per root owner session, bumped on mode changes/switches. */
    this.modeGens = new Map();
    /** Announced active ACP `cowboy` calls per session: sessionId -> Array<Entry>. */
    this.waiting = new Map();
    /** Facade call object -> captured turn at admission. */
    this.callTurns = new WeakMap();
    /** Facade call object -> `{ id, turn }` bound for UI progress/results. */
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
   * `sessionId` when `turn` is `undefined`).
   */
  clearTurn(sessionId, turn) {
    if (typeof sessionId !== "string") return;
    const list = this.waiting.get(sessionId);
    if (!list) return;
    if (turn === undefined) {
      this.waiting.delete(sessionId);
      return;
    }
    const left = list.filter((entry) => entry.turn !== turn);
    if (left.length) this.waiting.set(sessionId, left);
    else this.waiting.delete(sessionId);
  }

  /**
   * Record one `tool_call` or `tool_call_update` emitted by `agy` for UI
   * progress/result attachment. Terminal updates remove the entry.
   */
  trackUpdate(sessionId, update, turn) {
    const id = update?.toolCallId;
    if (typeof sessionId !== "string" || typeof id !== "string" || id === "") return;
    const list = this.waiting.get(sessionId) ?? [];
    if (TERMINAL.has(update.status)) {
      const left = list.filter((entry) => entry.id !== id);
      if (left.length) this.waiting.set(sessionId, left);
      else this.waiting.delete(sessionId);
      return;
    }
    // Never track new or updated announcements into a cancelled/closed attempt.
    if (turn !== undefined && !isTurnLive(turn)) return;
    const existing = list.find((entry) => entry.id === id);
    if (existing) {
      if (existing.turn !== turn) existing.ambiguous = true;
      if (typeof update.name === "string" && update.name !== existing.name) {
        existing.ambiguous = true;
      }
      if (update.rawInput !== undefined) {
        const stripSummary =
          PLAN_ALLOWED_TOOLS.has(existing.toolName) ||
          PLAN_REFUSED_TOOLS.has(existing.toolName) ||
          existing.toolName === "RunCommand";
        const digest = argsDigest(update.rawInput, { stripSummary });
        if (digest === undefined) existing.ambiguous = true;
        else if (!existing.hasInput) {
          existing.hasInput = true;
          existing.digest = digest;
        } else if (existing.digest !== digest) {
          existing.ambiguous = true;
        }
      }
      return;
    }
    const name = update.name;
    const toolName = stripFacadePrefix(name);
    if (toolName === undefined) return;
    const stripSummary =
      PLAN_ALLOWED_TOOLS.has(toolName) ||
      PLAN_REFUSED_TOOLS.has(toolName) ||
      toolName === "RunCommand";
    const hasRaw = update.rawInput !== undefined;
    const digest = hasRaw ? argsDigest(update.rawInput, { stripSummary }) : undefined;
    if (list.length >= MAX_WAITING) list.shift();
    list.push({
      id,
      name,
      toolName,
      turn,
      hasInput: hasRaw && digest !== undefined,
      digest,
      ambiguous: hasRaw && digest === undefined,
      boundCall: undefined,
    });
    this.waiting.set(sessionId, list);
  }

  /**
   * Capture `call`'s turn lifetime at `Facade` admission and eagerly bind it
   * to an unambiguous matching announcement in the same live turn.
   */
  admitCall(call, turn) {
    if (!call || typeof call !== "object") return;
    this.callTurns.set(call, turn);
    if (turn !== undefined && !isTurnLive(turn)) return;
    this.matchUnbound(call, turn);
  }

  /** Match `call` to an unbound waiting entry in the exact same `turn`. */
  matchUnbound(call, turn) {
    const list = this.waiting.get(call.sessionId) ?? [];
    const name = `${FACADE_PREFIX}${call.name}`;
    const stripSummary =
      PLAN_ALLOWED_TOOLS.has(call.name) ||
      PLAN_REFUSED_TOOLS.has(call.name) ||
      call.name === "RunCommand";
    const digest = argsDigest(call.args === undefined ? {} : call.args, { stripSummary });
    const unbound = list.filter(
      (item) => !item.boundCall && !item.ambiguous && item.name === name && item.turn === turn,
    );
    let entry;
    if (digest !== undefined) {
      entry = unbound.find((item) => item.hasInput && item.digest === digest);
      if (!entry) {
        const inputless = unbound.filter((item) => !item.hasInput);
        if (inputless.length === 1) entry = inputless[0];
      }
    } else if (unbound.length === 1) {
      entry = unbound[0];
    }
    if (!entry) return undefined;
    entry.boundCall = call;
    this.bound.set(call, { id: entry.id, turn });
    return entry.id;
  }

  /**
   * The ACP `toolCallId` one facade `call` answers for UI presentation
   * (`toolProgress`, `toolResult`). Enforces turn lifetime so late callbacks
   * after cancellation, turn settlement, or turn rollover never attach to a
   * different turn's call, and omits ambiguous matches.
   */
  bind(call, currentTurn) {
    if (!call || typeof call !== "object") return undefined;
    if (this.callTurns.has(call)) {
      const admittedTurn = this.callTurns.get(call);
      if (admittedTurn !== currentTurn) return undefined;
      if (admittedTurn !== undefined && !isTurnLive(admittedTurn)) return undefined;
    } else if (currentTurn !== undefined && !isTurnLive(currentTurn)) {
      return undefined;
    }
    if (this.bound.has(call)) {
      const binding = this.bound.get(call);
      if (binding.turn !== currentTurn) return undefined;
      if (binding.turn !== undefined && !isTurnLive(binding.turn)) return undefined;
      return binding.id;
    }
    return this.matchUnbound(call, currentTurn);
  }
}
