// Mode classification, mode generations, and attempt-scoped UI call correlation
// for Antigravity (R3).
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
const MAX_DIGEST_DEPTH = 64;

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

/** Whether top-level `toolSummary` is presentation-only for `toolName`. */
function isBuiltinFacadeTool(toolName) {
  return (
    PLAN_ALLOWED_TOOLS.has(toolName) ||
    PLAN_REFUSED_TOOLS.has(toolName) ||
    toolName === "RunCommand"
  );
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

/** Attempt identity for `turn` (distinguishes correction retries sharing one turn). */
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
    /** Facade call object -> `{ turn, attempt, admittedSeq }` captured at admission. */
    this.callAttempts = new WeakMap();
    /** Facade call object -> `{ entry, id, turn, attempt, digest }` bound for UI callbacks. */
    this.bound = new WeakMap();
    this.taintedCalls = new WeakSet();
    /** Monotonic sequence for ordering call admissions vs diff-card contradictions (R6-M2). */
    this.seq = 0;
    /**
     * Turn object -> Map<`${sessionId}\0${name}`, Map<attempt, { toolWide: number, byDigest: Map<string, number> }>>
     * Exact collisions taint only their digest; inputless/contradicted/evicted cards taint tool-wide (R7-L1).
     */
    this.turnAmbiguities = new WeakMap();
    this.unboundAmbiguities = new Map();
    /** Turn object -> Map<`${sessionId}\0${id}`, { aliasNames: Set<string>, attempt }> all evicted cards (R7-M3, R8-M4). */
    this.turnEvicted = new WeakMap();
    this.unboundEvicted = new Map();
  }

  ambiguityBucket(sessionId, turn, attempt, name, create = false) {
    let byKey;
    if (turn && typeof turn === "object") {
      byKey = this.turnAmbiguities.get(turn);
      if (!byKey) {
        if (!create) return undefined;
        byKey = new Map();
        this.turnAmbiguities.set(turn, byKey);
      }
    } else {
      byKey = this.unboundAmbiguities;
    }
    const key = `${sessionId}\0${name}`;
    let byAttempt = byKey.get(key);
    if (!byAttempt) {
      if (!create) return undefined;
      byAttempt = new Map();
      byKey.set(key, byAttempt);
    }
    let bucket = byAttempt.get(attempt);
    if (!bucket) {
      if (!create) return undefined;
      bucket = { toolWide: 0, byDigest: new Map() };
      byAttempt.set(attempt, bucket);
    }
    return bucket;
  }

  recordDiffAmbiguity(sessionId, turn, attempt, name, digest = undefined) {
    const seq = ++this.seq;
    const bucket = this.ambiguityBucket(sessionId, turn, attempt, name, true);
    if (digest === undefined) {
      bucket.toolWide = seq;
    } else {
      bucket.byDigest.set(digest, seq);
    }
  }

  latestDiffAmbiguity(sessionId, turn, attempt, name, digest) {
    const bucket = this.ambiguityBucket(sessionId, turn, attempt, name, false);
    if (!bucket) return 0;
    if (digest === undefined) {
      let best = bucket.toolWide;
      for (const seq of bucket.byDigest.values()) {
        if (seq > best) best = seq;
      }
      return best;
    }
    return Math.max(bucket.toolWide, bucket.byDigest.get(digest) ?? 0);
  }

  evictedMap(turn, create = false) {
    if (turn && typeof turn === "object") {
      let map = this.turnEvicted.get(turn);
      if (!map && create) {
        map = new Map();
        this.turnEvicted.set(turn, map);
      }
      return map;
    }
    return this.unboundEvicted;
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
   * `sessionId` when `turn` is `undefined`), returning unresolved `toolCallId`s
   * so subsequent correction attempts of the same turn can taint them.
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
   * existing entry (and its bound call's digest) BEFORE removing terminal
   * entries, returning `{ contradicted }` so the proxy can drop buffered
   * results for contradicted calls.
   */
  trackUpdate(sessionId, update, turn) {
    const id = update?.toolCallId;
    if (typeof sessionId !== "string" || typeof id !== "string" || id === "") {
      return { contradicted: false };
    }
    const list = this.waiting.get(sessionId) ?? [];
    const attempt = attemptOf(turn);
    const isDiffBearing = (toolName) => toolName === "WriteFile" || toolName === "EditFile";
    const isDiffName = (fullName) => isDiffBearing(stripFacadePrefix(fullName));
    const incomingName = typeof update.name === "string" ? update.name : undefined;
    const existing = list.find((entry) => entry.id === id);
    const evictedMap = this.evictedMap(turn, false);
    const evictedKey = `${sessionId}\0${id}`;
    const evictedRecord = !existing ? evictedMap?.get(evictedKey) : undefined;
    let contradicted = false;
    if (existing) {
      if (existing.turn !== turn || existing.attempt !== attempt) {
        existing.ambiguous = true;
        existing.inputContradicted = true;
        contradicted = true;
      }
      if (incomingName !== undefined && incomingName !== existing.name) {
        // R8-M3: Retain every revised identity on the card so later matching
        // peers and nameless completions remain tainted until retirement.
        existing.aliasNames.add(incomingName);
        existing.ambiguous = true;
        existing.inputContradicted = true;
        contradicted = true;
      }
      if (update.rawInput !== undefined) {
        const stripSummary = isBuiltinFacadeTool(existing.toolName);
        const digest = argsDigest(update.rawInput, { stripSummary });
        if (digest === undefined) {
          existing.ambiguous = true;
          existing.inputContradicted = true;
          contradicted = true;
        } else if (!existing.hasInput) {
          existing.hasInput = true;
          existing.digest = digest;
          if (existing.boundDigest !== undefined && existing.boundDigest !== digest) {
            existing.ambiguous = true;
            existing.inputContradicted = true;
            contradicted = true;
          }
        } else if (existing.digest !== digest) {
          existing.ambiguous = true;
          existing.inputContradicted = true;
          contradicted = true;
        }
      }
      if (existing.ambiguous) {
        contradicted = true;
        if (existing.boundCall) {
          this.taintedCalls.add(existing.boundCall);
          this.bound.delete(existing.boundCall);
          existing.boundCall = undefined;
        }
      }
    }
    const taintToolPeers = (peerName, targetTurn, targetAttempt, excludeId) => {
      const affected = [];
      if (!isDiffName(peerName)) return affected;
      this.recordDiffAmbiguity(sessionId, targetTurn, targetAttempt, peerName);
      for (const item of list) {
        if (
          item.id !== excludeId &&
          item.aliasNames.has(peerName) &&
          item.turn === targetTurn &&
          item.attempt === targetAttempt
        ) {
          item.ambiguous = true;
          if (item.boundCall) {
            this.taintedCalls.add(item.boundCall);
            this.bound.delete(item.boundCall);
            item.boundCall = undefined;
          }
          affected.push(item.id);
        }
      }
      return affected;
    };
    const detectDiffCollisions = (target) => {
      const affected = [];
      if (!target) return affected;
      const diffNames = [...target.aliasNames].filter(isDiffName);
      if (diffNames.length === 0) return affected;
      const toolWideTarget =
        Boolean(target.inputContradicted) || (target.ambiguous && !target.hasInput);
      for (const diffName of diffNames) {
        if (toolWideTarget) {
          this.recordDiffAmbiguity(sessionId, target.turn, target.attempt, diffName);
        } else if (target.ambiguous && target.hasInput && target.digest !== undefined) {
          this.recordDiffAmbiguity(sessionId, target.turn, target.attempt, diffName, target.digest);
        }
        const peers = list.filter(
          (item) =>
            item.id !== target.id &&
            item.aliasNames.has(diffName) &&
            item.turn === target.turn &&
            item.attempt === target.attempt,
        );
        const collides = (a, b) =>
          !a.hasInput ||
          !b.hasInput ||
          Boolean(a.inputContradicted) ||
          Boolean(b.inputContradicted) ||
          (a.digest !== undefined && a.digest === b.digest);
        const hits = peers.filter((peer) => collides(target, peer));
        if (hits.length > 0) {
          const exactOnly =
            target.hasInput &&
            !target.inputContradicted &&
            target.digest !== undefined &&
            hits.every(
              (peer) => peer.hasInput && !peer.inputContradicted && peer.digest === target.digest,
            );
          this.recordDiffAmbiguity(
            sessionId,
            target.turn,
            target.attempt,
            diffName,
            exactOnly ? target.digest : undefined,
          );
          for (const item of [target, ...hits]) {
            item.ambiguous = true;
            if (item.boundCall) {
              this.taintedCalls.add(item.boundCall);
              this.bound.delete(item.boundCall);
              item.boundCall = undefined;
            }
            affected.push(item.id);
          }
        }
      }
      return affected;
    };
    // R6-M1 / R7-M3 / R8-M3 / R8-M4: Propagate collisions across ALL retained
    // alias identities BEFORE removing terminal entries, and catch late/terminal
    // revisions on ANY evicted card (including evicted non-edit cards renamed to an edit).
    const existingAffected = existing ? detectDiffCollisions(existing) : [];
    if (evictedRecord) {
      if (incomingName !== undefined) evictedRecord.aliasNames.add(incomingName);
      const evictedDiffNames = [...evictedRecord.aliasNames].filter(isDiffName);
      if (evictedDiffNames.length > 0) {
        contradicted = true;
        for (const diffName of evictedDiffNames) {
          existingAffected.push(...taintToolPeers(diffName, turn, evictedRecord.attempt, id));
        }
      }
    }
    if (TERMINAL.has(update.status)) {
      const left = list.filter((entry) => entry.id !== id);
      if (left.length) this.waiting.set(sessionId, left);
      else this.waiting.delete(sessionId);
      if (evictedRecord) evictedMap?.delete(evictedKey);
      const contradictedIds = contradicted
        ? [...new Set([id, ...existingAffected])]
        : [...new Set(existingAffected)];
      return { contradicted: contradictedIds.length > 0, contradictedIds };
    }
    if (existing || evictedRecord) {
      const contradictedIds = contradicted
        ? [...new Set([id, ...existingAffected])]
        : [...new Set(existingAffected)];
      return { contradicted: contradictedIds.length > 0, contradictedIds };
    }
    if (turn !== undefined && !isTurnLive(turn))
      return { contradicted: false, contradictedIds: [] };
    const name = update.name;
    const toolName = stripFacadePrefix(name);
    if (toolName === undefined) return { contradicted: false, contradictedIds: [] };
    const stripSummary = isBuiltinFacadeTool(toolName);
    const hasRaw = update.rawInput !== undefined;
    const digest = hasRaw ? argsDigest(update.rawInput, { stripSummary }) : undefined;
    const tainted = Boolean(turn?.taintedToolCallIds?.has(id));
    let evictedId;
    if (list.length >= MAX_WAITING) {
      const evicted = list.shift();
      if (evicted) {
        evicted.ambiguous = true;
        evictedId = evicted.id;
        if (evicted.boundCall) {
          this.taintedCalls.add(evicted.boundCall);
          this.bound.delete(evicted.boundCall);
        }
        // R7-M3 / R8-M4: Retain ALL evicted card IDs (including non-edit cards)
        // so a late or terminal revision renaming an evicted card to an edit can
        // still invalidate surviving peers.
        const map = this.evictedMap(evicted.turn, true);
        if (map) {
          if (map.size >= MAX_WAITING * 4) {
            const oldest = map.keys().next().value;
            if (oldest !== undefined) map.delete(oldest);
          }
          map.set(`${sessionId}\0${evicted.id}`, {
            aliasNames: new Set(evicted.aliasNames),
            attempt: evicted.attempt,
          });
        }
        for (const diffName of [...evicted.aliasNames].filter(isDiffName)) {
          this.recordDiffAmbiguity(sessionId, evicted.turn, evicted.attempt, diffName);
        }
        if (turn && typeof turn === "object") turn.announcementEvicted = true;
      }
    }
    const added = {
      id,
      name,
      aliasNames: new Set([name]),
      toolName,
      turn,
      attempt,
      hasInput: hasRaw && digest !== undefined,
      digest,
      ambiguous: tainted || (hasRaw && digest === undefined),
      boundCall: undefined,
      boundDigest: undefined,
    };
    list.push(added);
    const affected = detectDiffCollisions(added);
    this.waiting.set(sessionId, list);
    return { contradicted: affected.length > 0, contradictedIds: affected, evictedId };
  }

  /**
   * Capture `call`'s turn and attempt lifetime at `Facade` admission and
   * eagerly bind it to an unambiguous matching announcement in the same live
   * attempt.
   */
  admitCall(call, turn) {
    if (!call || typeof call !== "object" || this.taintedCalls.has(call)) return;
    const attempt = attemptOf(turn);
    const prev = this.callAttempts.get(call);
    const admittedSeq =
      prev && prev.turn === turn && prev.attempt === attempt ? prev.admittedSeq : ++this.seq;
    this.callAttempts.set(call, { turn, attempt, admittedSeq });
    if (turn !== undefined && !isTurnLive(turn)) return;
    this.matchUnbound(call, turn, attempt);
  }

  /** Match `call` to an unbound waiting entry in the exact same `(turn, attempt)`. */
  matchUnbound(call, turn, attempt) {
    const list = this.waiting.get(call.sessionId) ?? [];
    const name = `${FACADE_PREFIX}${call.name}`;
    const stripSummary = isBuiltinFacadeTool(call.name);
    const digest = argsDigest(call.args === undefined ? {} : call.args, { stripSummary });
    const liveSameTool = list.filter(
      (item) => item.aliasNames.has(name) && item.turn === turn && item.attempt === attempt,
    );
    const unbound = liveSameTool.filter((item) => !item.boundCall && !item.ambiguous);
    let entry;
    const isDiffBearing = call.name === "WriteFile" || call.name === "EditFile";
    if (isDiffBearing) {
      // R6-M2: If a diff-bearing card for this tool/turn/attempt contradicted
      // itself or collided AFTER this unbound call was admitted (even if that
      // card already terminated before result delivery), permanently taint `call`.
      const admitted = this.callAttempts.get(call);
      if (
        admitted &&
        admitted.admittedSeq < this.latestDiffAmbiguity(call.sessionId, turn, attempt, name, digest)
      ) {
        this.taintedCalls.add(call);
        return undefined;
      }
      // R4-M5: If ANY compatible live edit announcement is already tainted
      // ambiguous (even if it is the sole remaining live card after a sibling
      // terminated), permanently taint `call` so it cannot later rebind.
      const compatibleLive =
        digest !== undefined
          ? liveSameTool.filter(
              (item) => !item.hasInput || Boolean(item.inputContradicted) || item.digest === digest,
            )
          : liveSameTool;
      if (compatibleLive.some((item) => item.ambiguous)) {
        this.taintedCalls.add(call);
        return undefined;
      }
    }
    if (digest !== undefined) {
      const exactLive = liveSameTool.filter((item) => item.hasInput && item.digest === digest);
      const inputlessLive = liveSameTool.filter((item) => !item.hasInput);
      const exactUnbound = unbound.filter((item) => item.hasInput && item.digest === digest);
      if (
        isDiffBearing &&
        (exactLive.length > 1 || (exactLive.length >= 1 && inputlessLive.length > 0))
      ) {
        // M-Proxy-6 & M13: Collisions across bound/unbound/inputless edit announcements taint all affected entries and calls permanently.
        this.taintedCalls.add(call);
        for (const item of [...exactLive, ...inputlessLive]) {
          item.ambiguous = true;
          if (item.boundCall) {
            this.taintedCalls.add(item.boundCall);
            this.bound.delete(item.boundCall);
            item.boundCall = undefined;
          }
        }
      } else if (exactUnbound.length >= 1) {
        if (!turn?.announcementEvicted) entry = exactUnbound[0];
      } else if (!turn?.announcementEvicted) {
        // R8-L1: Only taint on inputless fallback when an inputless live card actually exists.
        if (isDiffBearing && inputlessLive.length > 0 && liveSameTool.length > 1) {
          // R3-M6: Permanently taint the incoming call and colliding entries on inputless ambiguity.
          this.taintedCalls.add(call);
          for (const item of inputlessLive) {
            item.ambiguous = true;
            if (item.boundCall) {
              this.taintedCalls.add(item.boundCall);
              this.bound.delete(item.boundCall);
              item.boundCall = undefined;
            }
          }
        } else {
          const inputless = unbound.filter((item) => !item.hasInput);
          if (inputless.length === 1) entry = inputless[0];
        }
      }
    } else if (!turn?.announcementEvicted && unbound.length === 1) {
      if (!isDiffBearing || liveSameTool.length === 1) {
        entry = unbound[0];
      } else {
        this.taintedCalls.add(call);
      }
    } else if (isDiffBearing && liveSameTool.length > 1) {
      this.taintedCalls.add(call);
    }
    if (!entry) return undefined;
    entry.boundCall = call;
    entry.boundDigest = digest;
    this.bound.set(call, { entry, id: entry.id, turn, attempt, digest });
    return entry.id;
  }

  /**
   * The ACP `toolCallId` one facade `call` answers for UI presentation
   * (`toolProgress`, `toolResult`). Enforces `(turn, attempt)` lifetime,
   * suppresses bindings contradicted by later `tool_call_update` input, and
   * omits ambiguous matches.
   */
  bind(call, currentTurn) {
    if (!call || typeof call !== "object" || this.taintedCalls.has(call)) return undefined;
    const currentAttempt = attemptOf(currentTurn);
    if (this.callAttempts.has(call)) {
      const admitted = this.callAttempts.get(call);
      if (admitted.turn !== currentTurn || admitted.attempt !== currentAttempt) return undefined;
      if (admitted.turn !== undefined && !isTurnLive(admitted.turn)) return undefined;
    } else if (currentTurn !== undefined && !isTurnLive(currentTurn)) {
      return undefined;
    }
    if (this.bound.has(call)) {
      const binding = this.bound.get(call);
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
